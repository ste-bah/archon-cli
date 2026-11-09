//! Host-verified relevance and bounded independent equivalence answers.
use super::*;
use archon_workflow::breakage_proof::MutantRun;
use archon_workflow::breaker_verdict::rule::verified_quote;
const OUTPUT_BYTES: usize = 8 * 1024;
const FULL_IMAGE_BYTES: usize = 8 * 1024;
fn tail(text: &str) -> String {
    String::from_utf8_lossy(&text.as_bytes()[text.len().saturating_sub(OUTPUT_BYTES)..])
        .into_owned()
}
fn preimages(view: &BaseView, run: &MutantRun, patch: &str) -> serde_json::Value {
    let files = run.touched.iter().filter_map(|key| {
        let file = view.files.get(key)?;
        let text = String::from_utf8_lossy(&file.bytes);
        if file.bytes.len() <= FULL_IMAGE_BYTES {
            return Some(serde_json::json!({"key":key,"full_preimage":text}));
        }
        let mut current = false;
        let mut ranges = Vec::new();
        for line in patch.lines() {
            if line.starts_with("--- ") {
                current = line == format!("--- {}", rule::patch_path("a", key));
            }
            if current && line.starts_with("@@ ")
                && let Some(old) = line.split_whitespace().nth(1).and_then(|s|s.strip_prefix('-'))
            {
                let mut parts = old.split(',');
                if let Some(start) = parts.next().and_then(|s|s.parse::<usize>().ok()) {
                    let count = parts.next().and_then(|s|s.parse::<usize>().ok()).unwrap_or(1);
                    ranges.push((start.saturating_sub(1).saturating_sub(40), start.saturating_sub(1).saturating_add(count).saturating_add(40)));
                }
            }
        }
        let excerpts: Vec<_> = text.lines().enumerate()
            .filter(|(n,_)|ranges.iter().any(|(first,last)|n>=first && n<=last))
            .map(|(n,s)|format!("{}: {s}", n+1)).collect();
        Some(serde_json::json!({"key":key,"preimage_excerpts_within_40_lines_of_each_change":excerpts}))
    }).collect();
    serde_json::Value::Array(files)
}
pub(super) async fn relevance(
    ctx: &mut Context<'_>,
    pair: &mut PairVerdict,
    view: &BaseView,
    runs: &[MutantRun],
    store: Option<&Path>,
    text: &str,
) {
    let mut kills: Vec<_> = runs
        .iter()
        .filter(|r| r.counted && r.kind == MutantKind::Behaviour)
        .collect();
    kills.sort_by_key(|r| (&r.mutant_id, &r.check_id));
    for run in kills {
        if pair
            .relevance
            .iter()
            .any(|m| m.mutant_id == run.mutant_id && m.check_id == run.check_id)
        {
            continue;
        }
        if pair
            .relevance
            .iter()
            .any(|m| m.relevance == Relevance::Relevant)
        {
            break;
        }
        let Some(authored) = pair.mutants.iter().find(|m| m.mutant_id == run.mutant_id) else {
            continue;
        };
        let Some(check) = ctx
            .req
            .contract
            .acceptance
            .iter()
            .chain(&ctx.req.contract.supplementary)
            .find(|c| c.id == run.check_id)
        else {
            continue;
        };
        let named = named::check_named_keys(check, &ctx.probe.repository, &ctx.probe.project, view);
        let entry = ctx
            .req
            .classification
            .entries
            .iter()
            .find(|e| e.task_id == pair.task_id && e.id == pair.id)
            .expect("selected pair");
        let declared = selection::keys(
            entry,
            ctx.req
                .classification
                .base_snapshot
                .as_ref()
                .expect("view snapshot"),
            &ctx.probe.repository,
            false,
        );
        let named: Vec<_> = named
            .into_iter()
            .filter(|k| !selection::contains(&declared, k))
            .collect();
        let mut mark = RelevanceMark {
            mutant_id: run.mutant_id.clone(),
            check_id: run.check_id.clone(),
            relevance: Relevance::Unavailable,
            quote: None,
            reason: None,
            judge: ctx.verdicts.judges.relevance.clone(),
            cause: None,
        };
        if run.touched.iter().any(|k| named.contains(k)) {
            mark.relevance = Relevance::TouchesCheckInput;
            pair.relevance.push(mark);
            break;
        }
        let evidence = super::super::store::evidence_text_at(store, run);
        let (patch, output) = match evidence {
            Ok(e) => e,
            Err(e) => {
                mark.cause = Some(UnavailableCause::Transient);
                mark.reason = Some(e);
                pair.relevance.push(mark);
                break;
            }
        };
        let message = relevance_prompt(
            &pair.id,
            text,
            &authored.breaks,
            &patch,
            output.as_deref().unwrap_or(""),
        );
        let answer = answer(ctx, "breaker_relevance", &message, view).await;
        match answer {
            Ok(v) => {
                mark.quote = v.get("quote").and_then(|v| v.as_str()).map(str::to_string);
                mark.reason = v.get("reason").and_then(|v| v.as_str()).map(str::to_string);
                mark.relevance = match v.get("verdict").and_then(|v| v.as_str()) {
                    Some("relevant")
                        if mark.quote.as_deref().is_some_and(|q| {
                            verified_quote(text, q)
                                && (verified_quote(&authored.breaks, q)
                                    || verified_quote(q, &authored.breaks))
                        }) =>
                    {
                        Relevance::Relevant
                    }
                    Some("relevant") => Relevance::QuoteUnverified,
                    Some("not_relevant") => Relevance::NotRelevant,
                    _ => {
                        mark.cause = Some(UnavailableCause::Unusable);
                        Relevance::Unavailable
                    }
                };
            }
            Err((cause, reason)) => {
                mark.cause = Some(cause);
                mark.reason = Some(reason);
            }
        }
        pair.relevance.push(mark);
        break;
    }
}
async fn answer(
    ctx: &mut Context<'_>,
    role: &str,
    message: &str,
    view: &BaseView,
) -> Result<serde_json::Value, (UnavailableCause, String)> {
    if ctx.circuit.failed {
        return Err((UnavailableCause::Transient, "circuit_open".into()));
    }
    let default = if role == "breaker_equivalence_second" {
        SECOND_MODEL
    } else {
        FIRST_MODEL
    };
    let model = ctx.client.model_for_role(role, default);
    let key = cache::key(ctx.client, role, message, view, &model);
    if let Some(value) = cache::read(
        ctx.probe,
        ctx.store.as_deref(),
        &key,
        &mut ctx.verdicts.refusals,
    ) {
        if valid_answer(role, &value) {
            ctx.probe.resume.progress.reused(true);
            return Ok(value);
        }
        ctx.verdicts
            .refusals
            .push(format!("llm_cache_unreadable: {key}"));
    }
    for _ in 0..2 {
        let value = match ctx.circuit.judge(ctx.client, message, &model).await {
            Ok(v) => v,
            Err((cause @ (UnavailableCause::Transient | UnavailableCause::Budget), e)) => {
                return Err((cause, e));
            }
            Err(_) => continue,
        };
        let valid = valid_answer(role, &value);
        if valid {
            cache::save(
                ctx.probe,
                ctx.store.as_deref(),
                &key,
                value.clone(),
                &mut ctx.verdicts.refusals,
            );
            return Ok(value);
        }
    }
    Err((UnavailableCause::Unusable, "unusable_reply".into()))
}
pub(super) async fn equivalence(
    ctx: &mut Context<'_>,
    pair: &mut PairVerdict,
    view: &BaseView,
    runs: &[MutantRun],
    store: Option<&Path>,
    text: &str,
) {
    let mut survivors: Vec<_> = pair
        .mutants
        .iter()
        .filter(|m| m.kind == MutantKind::Behaviour && rule::survivor(&m.mutant_id, runs))
        .map(|m| m.mutant_id.clone())
        .collect();
    survivors.sort();
    let marks = pair.equivalence.iter().filter(|m| m.marked).count();
    for id in survivors {
        if pair.equivalence.iter().any(|m| m.mutant_id == id) {
            continue;
        }
        let Some(run) = runs
            .iter()
            .find(|r| r.mutant_id == id && matches!(r.outcome, RunOutcome::Survived { .. }))
        else {
            continue;
        };
        let evidence = super::super::store::evidence_text_at(store, run);
        let mut answers = Vec::new();
        let capped = marks >= MAX_EQUIVALENCE_MARKS;
        for (index, role) in ["breaker_equivalence", "breaker_equivalence_second"]
            .iter()
            .enumerate()
        {
            let mut result = JudgeAnswer {
                judge: ctx.verdicts.judges.equivalence[index].clone(),
                answer: EquivalenceAnswer::Unavailable,
                reason: None,
                cause: None,
            };
            if capped {
                result.cause = Some(UnavailableCause::Unusable);
                result.reason = Some("equivalence_cap_reached".into());
            } else if index == 1 && !ctx.verdicts.judges.equivalence_distinct {
                result.reason = Some("second_identity_not_distinct".into());
            } else if let Ok((patch, _)) = &evidence {
                let message = equivalence_prompt(&pair.id, text, view, run, patch);
                match answer(ctx, role, &message, view).await {
                    Ok(v) => {
                        result.answer = match v["verdict"].as_str() {
                            Some("equivalent") => EquivalenceAnswer::Equivalent,
                            Some("not_equivalent") => EquivalenceAnswer::NotEquivalent,
                            Some("unsure") => EquivalenceAnswer::Unsure,
                            _ => unreachable!("validated answer"),
                        };
                        result.reason = v["reason"].as_str().map(str::to_string);
                    }
                    Err((cause, reason)) => {
                        result.cause = Some(cause);
                        result.reason = Some(reason);
                    }
                }
            } else {
                result.cause = Some(UnavailableCause::Transient);
                result.reason = evidence.as_ref().err().cloned();
            }
            answers.push(result);
        }
        let marked = ctx.verdicts.judges.equivalence_distinct
            && !capped
            && answers
                .iter()
                .all(|a| a.answer == EquivalenceAnswer::Equivalent);
        if capped {
            ctx.verdicts.refusals.push(format!(
                "equivalence_cap_reached: {} {}",
                pair.task_id, pair.id
            ));
        }
        pair.equivalence.push(EquivalenceMark {
            mutant_id: id,
            first: answers.remove(0),
            second: answers.remove(0),
            marked,
            capped,
        });
        break;
    }
}

pub(super) fn relevance_prompt(
    id: &str,
    text: &str,
    breaks: &str,
    patch: &str,
    output: &str,
) -> String {
    format!(
        "Requirement {id}:\n{text}\nStated phrase: {breaks}\nChange:\n{patch}\nFailing output:\n{}\n1. Decide whether the change breaks a stated behaviour and the output shows that behaviour failing.\n2. Load, import, parse, setup and collection failures are not relevant.\n3. Copy an exact requirement phrase of at least three words into quote.\n4. Return JSON with verdict relevant or not_relevant, quote and reason.\nExample: {{\"verdict\":\"relevant\",\"quote\":\"exact requirement phrase\",\"reason\":\"The changed behaviour produces the wrong result.\"}}",
        tail(output)
    )
}
pub(super) fn equivalence_prompt(
    id: &str,
    text: &str,
    view: &BaseView,
    run: &MutantRun,
    patch: &str,
) -> String {
    format!(
        "Requirement {id}:\n{text}\nChange:\n{patch}\nOriginal content: {}\n1. Compare observable values for the same input and state.\n2. Different outputs mean not_equivalent, even when both outputs are constants.\n3. Answer equivalent only when every covered input and state gives the same observable result.\n4. Answer unsure when you cannot establish either result.\n5. Return one JSON object with verdict and reason.\nExample: {{\"verdict\":\"not_equivalent\",\"reason\":\"A covered input gives a different result.\"}}",
        preimages(view, run, patch)
    )
}

pub(super) fn valid_answer(role: &str, value: &serde_json::Value) -> bool {
    if role == "breaker_relevance" {
        matches!(value["verdict"].as_str(), Some("relevant" | "not_relevant"))
    } else {
        matches!(
            value["verdict"].as_str(),
            Some("equivalent" | "not_equivalent" | "unsure")
        )
    }
}

#[cfg(test)]
mod preimage_tests {
    use super::*;
    #[test]
    fn large_preimages_follow_each_actual_change_range() {
        let f = super::super::tests::fixture("unused");
        let mut view = materialise_view(
            f.parent.path(),
            f.source.path(),
            f.snapshot(),
            &[(
                "value".into(),
                archon_workflow::base_snapshot::SnapshotKind::Blob,
            )],
            &[],
        )
        .unwrap();
        let before = (0..10000)
            .map(|n| format!("line {n:05}\n"))
            .collect::<String>();
        view.files.get_mut("value").unwrap().bytes = before.as_bytes().to_vec();
        for line in [0, 9000, 9999] {
            let old = format!("line {line:05}");
            let after = before.replacen(&old, "changed", 1);
            let patch = rule::edit_patch("value", &before, &after);
            let run:MutantRun=serde_json::from_value(serde_json::json!({"check_id":"check","check_digest":"","control_key":"","mutant_id":"mutant","patch_digest":"","task_id":"task","requirement_id":"item","kind":"behaviour","touched":["value"],"outcome":{"outcome":"survived","elapsed_ms":0},"counted":false,"evidence":{"patch":patch}})).unwrap();
            let image = preimages(&view, &run, &patch).to_string();
            assert!(image.contains(&old));
            assert!(!image.contains("line 04000"));
            assert!(image.contains("preimage_excerpts"));
        }
    }
}
