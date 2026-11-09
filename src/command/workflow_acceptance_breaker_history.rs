//! Preserve unresolved survivor evidence for the same frozen inputs.
use super::*;
pub(super) fn load(ctx: &Context<'_>) -> Option<ProofVerdicts> {
    let path = ctx.store.as_ref()?.parent()?.join("survivor-history.json");
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let mut previous: ProofVerdicts = serde_json::from_value(record["evidence"].clone()).ok()?;
    previous.entries.retain(|p| {
        record["snapshots"][format!("{}:{}", p.task_id, p.id)]
            == pair_snapshot(ctx, &p.task_id, &p.id)
    });
    if let Some(root) = &ctx.store {
        if let Some(base) = record["base"]
            .as_str()
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            let source = root.parent()?.join(base);
            for pair in &previous.entries {
                for key in pair.patch_keys.values().chain(
                    pair.survivors
                        .iter()
                        .flat_map(|s| std::iter::once(&s.patch).chain(s.output.iter())),
                ) {
                    if archon_workflow::base_snapshot::outside_key(key) {
                        continue;
                    }
                    let dest = root.join(archon_workflow::base_snapshot::key_path(key));
                    if !dest.exists()
                        && let Ok(bytes) = std::fs::read(
                            source.join(archon_workflow::base_snapshot::key_path(key)),
                        )
                    {
                        if let Some(parent) = dest.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(dest, bytes);
                    }
                }
            }
        }
    }
    (previous.schema == VERDICT_SCHEMA).then_some(previous)
}
pub(super) fn restore(
    ctx: &Context<'_>,
    previous: &Option<ProofVerdicts>,
    pairs: &mut [PairVerdict],
) -> (BTreeSet<usize>, Vec<SuppliedMutant>) {
    let mut restored = BTreeSet::new();
    let mut supplied = Vec::new();
    if let Some(previous) = previous {
        for (index, pair) in pairs.iter_mut().enumerate() {
            if let Some(old) = previous.entries.iter().find(|p| {
                p.task_id == pair.task_id
                    && p.id == pair.id
                    && (!p.survivors.is_empty()
                        || matches!(
                            p.verdict,
                            Verdict::DoneCandidate
                                | Verdict::JudgeUnavailable
                                | Verdict::BudgetExhausted
                        ))
            }) {
                pair.attempts.extend(
                    old.attempts
                        .iter()
                        .filter(|a| {
                            a.reply_key.starts_with("body_gut:")
                                && (!a.refusals.is_empty()
                                    || matches!(a.outcome, AttemptOutcome::Unusable { .. }))
                        })
                        .cloned(),
                );
                for survivor in &old.survivors {
                    if !old
                        .mutants
                        .iter()
                        .any(|m| m.mutant_id == survivor.mutant_id)
                    {
                        missing(pair, &survivor.mutant_id, "authored_record_missing");
                    }
                }
                for authored in &old.mutants {
                    let key = old.patch_keys.get(&authored.mutant_id).or_else(|| {
                        old.survivors
                            .iter()
                            .find(|s| s.mutant_id == authored.mutant_id)
                            .map(|s| &s.patch)
                    });
                    let Some(key) = key.filter(|k| !archon_workflow::base_snapshot::outside_key(k))
                    else {
                        missing(pair, &authored.mutant_id, "patch_key_missing");
                        continue;
                    };
                    let Some(patch) = ctx.store.as_ref().and_then(|r| {
                        std::fs::read_to_string(
                            r.join(archon_workflow::base_snapshot::key_path(key)),
                        )
                        .ok()
                    }) else {
                        missing(pair, &authored.mutant_id, "patch_file_missing");
                        continue;
                    };
                    let m = SuppliedMutant {
                        task_id: pair.task_id.clone(),
                        requirement_id: pair.id.clone(),
                        kind: authored.kind,
                        patch,
                    };
                    if m.mutant_id() != authored.mutant_id {
                        missing(pair, &authored.mutant_id, "mutant_digest_mismatch");
                        continue;
                    }
                    pair.patch_keys
                        .insert(authored.mutant_id.clone(), key.clone());
                    pair.mutants.push(authored.clone());
                    supplied.push(m);
                    restored.insert(index);
                }
                ctx.probe.resume.progress.reused(restored.contains(&index));
            }
        }
    }
    (restored, supplied)
}
pub(super) fn save(ctx: &mut Context<'_>) {
    if let Some(root) = &ctx.store {
        let snapshots: BTreeMap<_, _> = ctx
            .verdicts
            .entries
            .iter()
            .map(|p| {
                (
                    format!("{}:{}", p.task_id, p.id),
                    pair_snapshot(ctx, &p.task_id, &p.id),
                )
            })
            .collect();
        let value = serde_json::json!({"logic":BREAKER_LOGIC,"base":ctx.verdicts.base_commit,"evidence":ctx.verdicts,"snapshots":snapshots});
        if super::super::store::atomic(
            &root
                .parent()
                .expect("evidence parent")
                .join("survivor-history.json"),
            &serde_json::to_vec_pretty(&value).expect("history"),
        )
        .is_err()
        {
            ctx.verdicts
                .refusals
                .push("survivor_history_not_saved".into());
        }
    }
}

pub(super) fn order(pairs: &[PairVerdict], previous: &Option<ProofVerdicts>) -> Vec<usize> {
    let mut order: Vec<_> = (0..pairs.len()).collect();
    order.sort_by_key(|&i| {
        let old = previous.as_ref().and_then(|p| {
            p.entries
                .iter()
                .find(|p| p.task_id == pairs[i].task_id && p.id == pairs[i].id)
        });
        match old.map(|p| p.verdict) {
            None => 0,
            _ if old.is_some_and(|p| {
                p.verdict == Verdict::BudgetExhausted
                    || p.run_budget_exhausted
                    || p.attempts.iter().any(|a| {
                        matches!(
                            a.outcome,
                            AttemptOutcome::Unavailable {
                                cause: UnavailableCause::Budget,
                                ..
                            }
                        )
                    })
                    || p.relevance
                        .iter()
                        .any(|m| m.cause == Some(UnavailableCause::Budget))
                    || p.equivalence.iter().any(|m| {
                        m.first.cause == Some(UnavailableCause::Budget)
                            || m.second.cause == Some(UnavailableCause::Budget)
                    })
            }) =>
            {
                0
            }
            Some(Verdict::DoneCandidate) => 2,
            _ => 1,
        }
    });
    order
}

fn pair_snapshot(ctx: &Context<'_>, task: &str, id: &str) -> String {
    let Some(snapshot) = ctx.req.classification.base_snapshot.as_ref() else {
        return String::new();
    };
    let Some(entry) = ctx
        .req
        .classification
        .entries
        .iter()
        .find(|e| e.task_id == task && e.id == id)
    else {
        return String::new();
    };
    let keys = selection::keys(entry, snapshot, &ctx.probe.repository, false);
    let objects: BTreeMap<_, _> = snapshot
        .entries
        .iter()
        .filter(|(k, _)| selection::contains(&keys, k))
        .collect();
    content_digest(&serde_json::to_vec(&objects).expect("snapshot objects"))
}
pub(super) fn retain_unrun(
    previous: &Option<ProofVerdicts>,
    pairs: &mut [PairVerdict],
    all: &[MutantRun],
) {
    let Some(previous) = previous else {
        return;
    };
    for pair in pairs {
        if let Some(old) = previous
            .entries
            .iter()
            .find(|p| p.task_id == pair.task_id && p.id == pair.id)
        {
            for s in &old.survivors {
                if !pair.mutants.iter().any(|m| m.mutant_id == s.mutant_id) {
                    continue;
                }
                if let Some(run) = all.iter().find(|r| {
                    r.task_id == pair.task_id
                        && r.requirement_id == pair.id
                        && r.mutant_id == s.mutant_id
                        && matches!(
                            r.outcome,
                            RunOutcome::Unviable { .. } | RunOutcome::Inapplicable { .. }
                        )
                }) {
                    missing(
                        pair,
                        &s.mutant_id,
                        &format!("survivor_unviable: {:?}", run.outcome),
                    );
                    pair.mutants.retain(|m| m.mutant_id != s.mutant_id);
                    pair.patch_keys.remove(&s.mutant_id);
                    continue;
                }
                if !pair.survivors.iter().any(|v| v.mutant_id == s.mutant_id)
                    && !pair
                        .equivalence
                        .iter()
                        .any(|m| m.mutant_id == s.mutant_id && m.marked)
                    && !all.iter().any(|r| {
                        r.task_id == pair.task_id
                            && r.requirement_id == pair.id
                            && r.mutant_id == s.mutant_id
                            && (r.counted || matches!(r.outcome, RunOutcome::Survived { .. }))
                    })
                {
                    pair.survivors.push(s.clone());
                }
            }
        }
    }
}

pub(super) fn needs_gut(previous: &Option<ProofVerdicts>, pair: &PairVerdict) -> bool {
    !pair.mutants.iter().any(|m| m.kind == MutantKind::BodyGut)
        && previous.as_ref().is_some_and(|p| {
            p.entries.iter().any(|old| {
                old.task_id == pair.task_id
                    && old.id == pair.id
                    && old.attempts.iter().any(|a| {
                        a.reply_key.starts_with("body_gut:")
                            && matches!(
                                a.outcome,
                                AttemptOutcome::Unavailable {
                                    cause: UnavailableCause::Budget | UnavailableCause::Transient,
                                    ..
                                }
                            )
                    })
            })
        })
}

fn missing(pair: &mut PairVerdict, id: &str, reason: &str) {
    pair.attempts.push(AttemptRecord {
        attempt: 0,
        reply_key: "history".into(),
        outcome: AttemptOutcome::Discarded {
            reason: reason.into(),
        },
        refusals: vec![format!("survivor_dropped: {id}: {reason}")],
    });
}
