use super::*;
fn feedback(pair: &PairVerdict, runs: &[MutantRun]) -> String {
    let mut out = Vec::new();
    for attempt in pair.attempts.iter().filter(|a| a.reply_key != "history") {
        out.extend(
            attempt
                .refusals
                .iter()
                .map(|r| format!("Previous proposal refused: {r}")),
        );
        if let AttemptOutcome::Unusable { reason } = &attempt.outcome {
            out.push(format!("Previous reply unusable: {reason}"));
        }
    }
    for m in &pair.mutants {
        for run in runs.iter().filter(|r| {
            r.mutant_id == m.mutant_id && r.task_id == pair.task_id && r.requirement_id == pair.id
        }) {
            let detail = match &run.outcome {
                RunOutcome::Inapplicable { code, .. } => Some(code.clone()),
                RunOutcome::Unviable { located } => Some(
                    located
                        .iter()
                        .filter(|l| l.class == LocatedClass::Mutated)
                        .map(|l| format!("{}:{}", l.path, l.line))
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                _ if pair.relevance.iter().any(|r| {
                    r.mutant_id == m.mutant_id
                        && matches!(
                            r.relevance,
                            Relevance::NotRelevant
                                | Relevance::QuoteUnverified
                                | Relevance::TouchesCheckInput
                        )
                }) =>
                {
                    Some("did not break a behaviour the requirement states".into())
                }
                _ => None,
            };
            if let Some(detail) = detail {
                out.push(format!("Phrase {:?}: {detail}", m.breaks));
            }
        }
    }
    out.sort();
    out.dedup();
    out.join("\n")
}
pub(super) async fn author(
    ctx: &mut Context<'_>,
    index: usize,
    pair: &mut PairVerdict,
    attempt: u32,
    runs: &[MutantRun],
) -> Vec<SuppliedMutant> {
    let Some(text) = rule::requirement_text(ctx.req.prd_text, &pair.id) else {
        return vec![];
    };
    let mut record = AttemptRecord {
        attempt,
        reply_key: String::new(),
        outcome: AttemptOutcome::Unusable {
            reason: "no_accepted_mutants".into(),
        },
        refusals: vec![],
    };
    if ctx.circuit.failed {
        record.outcome = AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            reason: "circuit_open".into(),
        };
        pair.attempts.push(record);
        return vec![];
    }
    let mut view = match view(ctx, index).await {
        Ok(v) => v,
        Err(e) => {
            record.refusals.push(e.clone());
            record.outcome = if e == "view_too_large" {
                AttemptOutcome::Unusable { reason: e }
            } else {
                AttemptOutcome::Unavailable {
                    cause: UnavailableCause::Transient,
                    reason: e,
                }
            };
            pair.attempts.push(record);
            return vec![];
        }
    };
    let keys = selection::passing_checks(
        &ctx.req.classification.entries[index],
        ctx.req.contract,
        ctx.req.lock,
    )
    .iter()
    .filter_map(|id| {
        ctx.req
            .contract
            .acceptance
            .iter()
            .chain(&ctx.req.contract.supplementary)
            .find(|c| &c.id == id)
    })
    .flat_map(|c| named::check_named_keys(c, &ctx.probe.repository, &ctx.probe.project, &view))
    .collect::<Vec<_>>();
    let declared = selection::keys(
        &ctx.req.classification.entries[index],
        ctx.req
            .classification
            .base_snapshot
            .as_ref()
            .expect("view snapshot"),
        &ctx.probe.repository,
        false,
    );
    let keys: Vec<_> = keys
        .into_iter()
        .filter(|k| !selection::contains(&declared, k))
        .collect();
    if let Err(e) = named::exclude(&mut view, &keys) {
        record.outcome = AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            reason: e,
        };
        pair.attempts.push(record);
        return vec![];
    }
    if view.files.is_empty() {
        pair.verdict = Verdict::NoCountableDeliverable;
        return vec![];
    }
    let message = prompt::prompt(&pair.id, &text, &view, &feedback(pair, runs));
    let model = ctx.client.model_for_role("breaker", JUDGE_MODEL_ALIAS);
    let reply_key = cache::key(ctx.client, "breaker", &message, &view, &model);
    let key = content_digest(format!("{reply_key}:{attempt}").as_bytes());
    record.reply_key = key.clone();
    let cached = {
        cache::read(
            ctx.probe,
            ctx.store.as_deref(),
            &key,
            &mut ctx.verdicts.refusals,
        )
    };
    let present = cached.is_some();
    let cached = cached.and_then(|v| {
        let text_reply = v.as_str()?;
        prompt::parse(text_reply, &pair.task_id, &pair.id, &text, &view)
            .ok()
            .filter(|p| !p.mutants.is_empty())
            .map(|_| text_reply.to_string())
    });
    if cached.is_none() && present {
        ctx.verdicts
            .refusals
            .push(format!("llm_cache_unreadable: {key}"));
    }
    let reply = match cached {
        Some(s) => {
            ctx.probe.resume.progress.reused(true);
            Ok(s)
        }
        None => {
            ctx.circuit
                .author(ctx.client, &view, &message, &model, attempt)
                .await
        }
    };
    let mut supplied = vec![];
    match reply {
        Ok(reply) => match prompt::parse(&reply, &pair.task_id, &pair.id, &text, &view) {
            Ok(parsed) => {
                record.refusals = parsed.refusals;
                for (mutant, breaks) in parsed.mutants {
                    if mutant.kind != MutantKind::Behaviour {
                        record
                            .refusals
                            .push("body_gut_requires_separate_call".into());
                        continue;
                    }
                    let id = mutant.mutant_id();
                    let authored = AuthoredMutant {
                        mutant_id: id,
                        kind: mutant.kind,
                        breaks,
                        attempt,
                    };
                    record_authored(pair, authored);
                    supplied.push(mutant);
                }
                if !supplied.is_empty() {
                    record.outcome = AttemptOutcome::Accepted {
                        mutants: supplied.iter().map(|m| m.mutant_id()).collect(),
                    };
                    ctx.pending.push((index, key, reply));
                }
            }
            Err(reason) => record.outcome = AttemptOutcome::Unusable { reason },
        },
        Err((UnavailableCause::Unusable, reason)) => {
            record.outcome = AttemptOutcome::Unusable { reason }
        }
        Err((cause, reason)) => record.outcome = AttemptOutcome::Unavailable { cause, reason },
    }
    if !ctx.circuit.failed
        && !pair
            .attempts
            .iter()
            .any(|a| a.reply_key.starts_with("body_gut:"))
    {
        supplied.extend(super::gut::author(ctx, pair, &view, &text, "", 1).await);
    }
    if let Err(e) = view.remove() {
        ctx.verdicts
            .refusals
            .push(format!("view_remove_failed: {e}"));
    }
    pair.attempts.push(record);
    supplied
}

// A repeated patch classified as a body removal must never acquire an
// equivalence exemption from an earlier behaviour description.
pub(super) fn record_authored(pair: &mut PairVerdict, authored: AuthoredMutant) {
    if let Some(previous) = pair
        .mutants
        .iter_mut()
        .find(|m| m.mutant_id == authored.mutant_id)
    {
        if authored.kind == MutantKind::BodyGut {
            *previous = authored;
        }
    } else {
        pair.mutants.push(authored);
    }
}
