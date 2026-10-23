use std::collections::BTreeMap;
use std::collections::BTreeSet;

use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};

#[derive(Default)]
pub(super) struct JudgeRetryCandidates(BTreeMap<String, AcceptanceCriterion>);

impl JudgeRetryCandidates {
    pub(super) fn take(&mut self, id: &str) -> Option<AcceptanceCriterion> {
        self.0.remove(id)
    }

    pub(super) fn remember_contract(&mut self, contract: &AcceptanceContract) {
        self.0.extend(
            contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .cloned()
                .map(|entry| (entry.id.clone(), entry)),
        );
    }
}

pub(super) fn candidate_ids(contract: &AcceptanceContract) -> Vec<String> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect()
}

/// Authored passability-judge replies stay pending per affected check.
pub(super) fn authored_passability_pending(
    error: &anyhow::Error,
    ids: &BTreeSet<String>,
) -> Option<(BTreeSet<String>, String)> {
    crate::command::workflow_freeze_budget::FreezeIncomplete::caused(error)
        .filter(|incomplete| incomplete.is_authored_judge_reply())
        .map(|incomplete| {
            // Only the checks the unusable reply left without a verdict;
            // every other sound check keeps its own outcome.
            let affected: BTreeSet<String> = (incomplete.deferred().iter())
                .filter(|id| ids.contains(*id))
                .cloned()
                .collect();
            (
                if affected.is_empty() {
                    ids.clone()
                } else {
                    affected
                },
                format!(
                    "the passability judge returned no usable authored verdict: {error:#}; this check remains pending for judge retry"
                ),
            )
        })
}
