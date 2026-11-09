use super::*;
pub(super) fn supplements(
    ctx: &mut Context<'_>,
    pairs: &mut [PairVerdict],
    all: &[MutantRun],
    distinct: bool,
) {
    for pair in pairs.iter_mut() {
        if let Some(root) = &ctx.store {
            for run in all
                .iter()
                .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
            {
                if !archon_workflow::base_snapshot::outside_key(&run.evidence.patch)
                    && root
                        .join(archon_workflow::base_snapshot::key_path(
                            &run.evidence.patch,
                        ))
                        .is_file()
                {
                    pair.patch_keys
                        .insert(run.mutant_id.clone(), run.evidence.patch.clone());
                }
            }
        }
        let retained = std::mem::take(&mut pair.survivors);
        pair.survivors = pair
            .mutants
            .iter()
            .filter(|m| {
                rule::survivor(
                    &m.mutant_id,
                    all.iter()
                        .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id),
                ) && !pair
                    .equivalence
                    .iter()
                    .any(|e| e.mutant_id == m.mutant_id && e.marked)
            })
            .filter_map(|m| {
                let r = all.iter().find(|r| {
                    r.task_id == pair.task_id
                        && r.requirement_id == pair.id
                        && r.mutant_id == m.mutant_id
                        && matches!(r.outcome, RunOutcome::Survived { .. })
                })?;
                Some(SurvivorEvidence {
                    mutant_id: m.mutant_id.clone(),
                    kind: m.kind,
                    checks: all
                        .iter()
                        .filter(|r| {
                            r.task_id == pair.task_id
                                && r.requirement_id == pair.id
                                && r.mutant_id == m.mutant_id
                                && matches!(r.outcome, RunOutcome::Survived { .. })
                        })
                        .map(|r| r.check_id.clone())
                        .collect(),
                    patch: r.evidence.patch.clone(),
                    output: r.evidence.output.clone(),
                    equivalence_unjudged: m.kind == MutantKind::Behaviour
                        && pair
                            .equivalence
                            .iter()
                            .find(|mark| mark.mutant_id == m.mutant_id)
                            .is_none_or(|mark| {
                                mark.first.answer == EquivalenceAnswer::Unavailable
                                    || distinct
                                        && mark.second.answer == EquivalenceAnswer::Unavailable
                            }),
                })
            })
            .collect();
        for survivor in retained {
            if !pair
                .survivors
                .iter()
                .any(|s| s.mutant_id == survivor.mutant_id)
            {
                pair.survivors.push(survivor);
            }
        }
    }
    let mut used: BTreeSet<_> = ctx
        .req
        .contract
        .acceptance
        .iter()
        .chain(&ctx.req.contract.supplementary)
        .map(|c| c.id.clone())
        .collect();
    pairs.sort_by(|a, b| (&a.task_id, &a.id).cmp(&(&b.task_id, &b.id)));
    for pair in pairs.iter_mut() {
        if pair.verdict != Verdict::WeakTest {
            continue;
        }
        match rule::mint(&pair.id, &mut used) {
            Ok(sup_id) => {
                let survivors = pair.survivors.clone();
                pair.owed = Some(OwedSupplement {
                    sup_id,
                    requirement_id: pair.id.clone(),
                    task_id: pair.task_id.clone(),
                    survivors,
                });
            }
            Err(e) => ctx.verdicts.refusals.push(e),
        }
    }
}

pub(super) fn decisions(
    pairs: &mut [PairVerdict],
    states: &[Vec<ProofState>],
    all: &[MutantRun],
    refused: bool,
    distinct: bool,
) {
    for (index, pair) in pairs.iter_mut().enumerate() {
        if pair.verdict == Verdict::NoCountableDeliverable {
            continue;
        }
        let runs: Vec<_> = all
            .iter()
            .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
            .cloned()
            .collect();
        pair.run_budget_exhausted |= states[index].contains(&ProofState::BudgetExhausted)
            || runs
                .iter()
                .any(|r| matches!(r.outcome, RunOutcome::BudgetExhausted));
        pair.verdict = rule::decide(rule::Decision {
            refused,
            has_text: pair.requirement_text_digest.is_some(),
            states: &states[index],
            pair,
            runs: &runs,
            distinct,
        });
    }
}
