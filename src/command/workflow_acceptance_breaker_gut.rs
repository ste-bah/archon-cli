//! Focused body removal requests, one target and one retry at a time.
use super::*;
pub(super) async fn author(
    ctx: &mut Context<'_>,
    pair: &mut PairVerdict,
    view: &BaseView,
    text: &str,
    feedback: &str,
    start: u32,
) -> Vec<SuppliedMutant> {
    let mut supplied = Vec::new();
    let model = ctx.client.model_for_role("breaker", JUDGE_MODEL_ALIAS);
    {
        let declared = ctx
            .req
            .classification
            .entries
            .iter()
            .find(|e| e.task_id == pair.task_id && e.id == pair.id)
            .zip(ctx.req.classification.base_snapshot.as_ref())
            .map(|(e, s)| selection::keys(e, s, &ctx.probe.repository, false))
            .unwrap_or_default();
        let keys: Vec<_> = view
            .files
            .keys()
            .filter(|k| selection::contains(&declared, k))
            .cloned()
            .collect();
        let key = serde_json::to_string(&keys).expect("file keys");
        let mut feedback = feedback.to_string();
        for attempt in start..=start.max(2) {
            let message = prompt::gut_prompt(text, view, &key, &feedback);
            let reply_key = cache::key(ctx.client, "body_gut", &message, view, &model);
            let cached = cache::read(
                ctx.probe,
                ctx.store.as_deref(),
                &reply_key,
                &mut ctx.verdicts.refusals,
            )
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|r| prompt::parse_gut(r, &pair.task_id, &pair.id, text, view, &key).is_ok());
            let reply = match cached {
                Some(reply) => {
                    ctx.probe.resume.progress.reused(true);
                    Ok(reply)
                }
                None => {
                    ctx.circuit
                        .author(ctx.client, view, &message, &model, attempt)
                        .await
                }
            };
            let mut record = AttemptRecord {
                attempt,
                reply_key: format!("body_gut:{reply_key}"),
                outcome: AttemptOutcome::Unusable {
                    reason: String::new(),
                },
                refusals: vec![],
            };
            match reply {
                Ok(reply) => {
                    match prompt::parse_gut(&reply, &pair.task_id, &pair.id, text, view, &key) {
                        Ok(parsed) => {
                            cache::save(
                                ctx.probe,
                                ctx.store.as_deref(),
                                &reply_key,
                                serde_json::Value::String(reply.clone()),
                                &mut ctx.verdicts.refusals,
                            );
                            record.refusals = parsed.refusals;
                            let ids = parsed.mutants.iter().map(|(m, _)| m.mutant_id()).collect();
                            for (m, breaks) in parsed.mutants {
                                author::record_authored(
                                    pair,
                                    AuthoredMutant {
                                        mutant_id: m.mutant_id(),
                                        kind: m.kind,
                                        breaks,
                                        attempt,
                                    },
                                );
                                supplied.push(m);
                            }
                            record.outcome = AttemptOutcome::Accepted { mutants: ids };
                        }
                        Err(reason) => {
                            feedback = format!("Previous reply failed: {reason}");
                            record.outcome = AttemptOutcome::Unusable { reason };
                        }
                    }
                }
                Err((UnavailableCause::Unusable, reason)) => {
                    feedback = format!("Previous reply failed: {reason}");
                    record.outcome = AttemptOutcome::Unusable { reason };
                }
                Err((cause, reason)) => {
                    record.outcome = AttemptOutcome::Unavailable { cause, reason };
                }
            }
            let accepted = matches!(record.outcome, AttemptOutcome::Accepted { .. });
            let unavailable = matches!(record.outcome, AttemptOutcome::Unavailable { .. });
            pair.attempts.push(record);
            if accepted || unavailable {
                break;
            }
        }
    }
    supplied
}

pub(super) async fn resume(
    ctx: &mut Context<'_>,
    index: usize,
    pair: &mut PairVerdict,
) -> Vec<SuppliedMutant> {
    let Some(text) = rule::requirement_text(ctx.req.prd_text, &pair.id) else {
        return vec![];
    };
    match super::view(ctx, index).await {
        Ok(view) => {
            let supplied = author(ctx, pair, &view, &text, "", 1).await;
            if let Err(e) = view.remove() {
                ctx.verdicts
                    .refusals
                    .push(format!("view_remove_failed: {e}"));
            }
            supplied
        }
        Err(reason) => {
            pair.attempts.push(AttemptRecord {
                attempt: 1,
                reply_key: "body_gut:resume".into(),
                outcome: AttemptOutcome::Unavailable {
                    cause: UnavailableCause::Transient,
                    reason,
                },
                refusals: vec![],
            });
            vec![]
        }
    }
}

pub(super) async fn retry(
    ctx: &mut Context<'_>,
    index: usize,
    pair: &mut PairVerdict,
    runs: &[MutantRun],
) -> Vec<SuppliedMutant> {
    let attempts = pair
        .attempts
        .iter()
        .filter(|a| {
            a.reply_key.starts_with("body_gut:")
                && matches!(a.outcome, AttemptOutcome::Accepted { .. })
        })
        .count();
    if attempts != 1 {
        return vec![];
    }
    let failed = runs.iter().find(|r| {
        r.task_id == pair.task_id
            && r.requirement_id == pair.id
            && r.kind == MutantKind::BodyGut
            && matches!(
                r.outcome,
                RunOutcome::Unviable { .. } | RunOutcome::Inapplicable { .. }
            )
    });
    let Some(run) = failed else {
        return vec![];
    };
    let reason = failure_reason(run).expect("failed removal");
    let Some(text) = rule::requirement_text(ctx.req.prd_text, &pair.id) else {
        return vec![];
    };
    let Ok(view) = super::view(ctx, index).await else {
        return vec![];
    };
    let start = pair
        .attempts
        .iter()
        .filter(|a| a.reply_key.starts_with("body_gut:"))
        .map(|a| a.attempt)
        .max()
        .unwrap_or(0)
        + 1;
    let supplied = author(
        ctx,
        pair,
        &view,
        &text,
        &format!("Previous reply failed: {reason}"),
        start,
    )
    .await;
    if let Err(e) = view.remove() {
        ctx.verdicts
            .refusals
            .push(format!("view_remove_failed: {e}"));
    }
    supplied
}

fn failure_reason(run: &MutantRun) -> Option<String> {
    match &run.outcome {
        RunOutcome::Inapplicable { code, .. } => Some(code.clone()),
        RunOutcome::Unviable { located } => {
            let reason = located
                .iter()
                .filter(|l| l.class == LocatedClass::Mutated)
                .map(|l| format!("{}:{}", l.path, l.line))
                .collect::<Vec<_>>()
                .join(",");
            Some(if reason.is_empty() {
                "unviable: no changed-file diagnostics".into()
            } else {
                reason
            })
        }
        _ => None,
    }
}
pub(super) fn record_failures(pairs: &mut [PairVerdict], runs: &[MutantRun]) {
    for pair in pairs {
        for run in runs.iter().filter(|r| {
            r.task_id == pair.task_id
                && r.requirement_id == pair.id
                && r.kind == MutantKind::BodyGut
        }) {
            let Some(reason) = failure_reason(run) else {
                continue;
            };
            for attempt in &mut pair.attempts {
                if attempt.reply_key.starts_with("body_gut:")
                    && matches!(&attempt.outcome, AttemptOutcome::Accepted { mutants } if mutants.contains(&run.mutant_id))
                    && !attempt.refusals.contains(&reason)
                {
                    attempt.refusals.push(reason.clone());
                }
            }
        }
    }
}
