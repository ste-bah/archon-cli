//! Per-criterion best-of across acceptance attempts.
//!
//! An author told to fix one refuted check tends to rewrite the whole
//! contract, and a live phase went 22, 1, 22, 1, 11 findings and spent its
//! budget one check short. The host keeps what it has already accepted: a
//! criterion the new candidate gets wrong, where the freeze on disk carries
//! the same criterion judged accepted and clean, keeps the accepted entry and
//! its stored verdict. Progress becomes monotone per criterion; the judge is
//! never asked twice about the same check; nothing here knows a PRD.
use std::collections::BTreeSet;

use archon_workflow::task_set_contract::{
    AcceptanceContract, JudgeDecision, acceptance_policy_findings,
};

/// Replace every defective criterion in `contract` with the accepted entry of
/// the same id from `base` when both describe the same PRD and criterion.
/// Returns the ids replaced.
pub(super) fn keep_previously_accepted(
    contract: &mut AcceptanceContract,
    base: &AcceptanceContract,
) -> Vec<String> {
    if contract.prd.digest != base.prd.digest {
        return Vec::new();
    }
    let defective = defective_ids(contract);
    let base_clean: BTreeSet<String> = base
        .acceptance
        .iter()
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>()
        .difference(&defective_ids(base))
        .cloned()
        .collect();
    let mut replaced = Vec::new();
    for entry in &mut contract.acceptance {
        if !defective.contains(&entry.id) || !base_clean.contains(&entry.id) {
            continue;
        }
        let Some(accepted) = base.acceptance.iter().find(|b| b.id == entry.id) else {
            continue;
        };
        // The candidate's gap declaration already agrees with its own gap
        // policy (structure is validated first); an entry carrying the other
        // declaration would fail bundle validation at publish.
        if accepted.criterion != entry.criterion || accepted.gap_permitted != entry.gap_permitted {
            continue;
        }
        *entry = accepted.clone();
        replaced.push(entry.id.clone());
    }
    replaced
}

/// Ids the host or the judge found fault with.
fn defective_ids(contract: &AcceptanceContract) -> BTreeSet<String> {
    let mut ids: BTreeSet<String> = acceptance_policy_findings(contract)
        .into_iter()
        .filter_map(|finding| finding.field.split('.').next().map(str::to_string))
        .collect();
    ids.extend(
        contract
            .acceptance
            .iter()
            .filter(|entry| entry.judgment.verdict != JudgeDecision::Accepted)
            .map(|entry| entry.id.clone()),
    );
    ids
}

#[cfg(test)]
#[path = "workflow_task_set_merge_tests.rs"]
mod tests;
