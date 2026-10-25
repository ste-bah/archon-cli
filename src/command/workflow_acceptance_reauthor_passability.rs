//! What a judge or passability judge that could not complete leaves of the
//! candidates it was asked about (D2, D3): each stays pending by whose
//! fault it is -- the judge's own (an unusable reply, a refusal) for another
//! judge attempt, a provider or host fault for the host, resumably -- and no
//! candidate it was not asked about is touched, so a sibling that proved
//! itself is still published.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};

use crate::command::workflow_freeze_budget::FreezeIncomplete;
use crate::command::workflow_task_set::judge::JudgeIncomplete;

#[cfg(test)]
#[path = "workflow_acceptance_reauthor_judge_partial_tests.rs"]
mod judge_partial_tests;

#[derive(Default)]
pub(super) struct JudgeRetryCandidates(BTreeMap<String, AcceptanceCriterion>);

impl JudgeRetryCandidates {
    pub(super) fn take(&mut self, id: &str) -> Option<AcceptanceCriterion> {
        self.0.remove(id)
    }
}

/// The ids, pending lists and host-pending findings a held candidate
/// leaves: (`pending`, `reused`, `unproven`) of the re-author loop.
pub(super) type Pending<'a> = (
    &'a mut BTreeSet<String>,
    &'a mut BTreeMap<String, AcceptanceCriterion>,
    &'a mut BTreeMap<String, String>,
);

/// Hold undecided entries of `asked`, the judge call `error` ended: for its
/// author's next judge attempt when the judge itself gave no usable verdict
/// (each note in `feedback`), else for the host (`pending`). Return decided
/// entries for normal verdict handling. A non-incomplete error is unchanged.
pub(super) fn hold_judge_failure(
    error: anyhow::Error,
    asked: &AcceptanceContract,
    retry: &mut JudgeRetryCandidates,
    feedback: &mut BTreeMap<String, Vec<String>>,
    (pending, reused, unproven): Pending<'_>,
) -> anyhow::Result<AcceptanceContract> {
    let Some(incomplete) = JudgeIncomplete::caused(&error) else {
        return Err(error);
    };
    let authored = incomplete.is_authored_reply();
    let finding = if authored {
        format!(
            "the acceptance judge returned no usable authored verdict: {error:#}; this check remains pending for re-author and judge retry"
        )
    } else {
        format!(
            "the acceptance judge could not complete for a provider or host fault: {error:#}; this check remains pending for the host and is judged again on resume"
        )
    };
    let decided_ids: BTreeSet<String> = incomplete
        .decided
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let mut decided = asked.clone();
    decided
        .acceptance
        .retain(|entry| decided_ids.contains(&entry.id));
    decided
        .supplementary
        .retain(|entry| decided_ids.contains(&entry.id));
    for judged_entry in &incomplete.decided {
        if let Some(entry) = decided
            .acceptance
            .iter_mut()
            .chain(&mut decided.supplementary)
            .find(|entry| entry.id == judged_entry.id)
        {
            *entry = judged_entry.clone();
        }
    }
    for entry in asked.acceptance.iter().chain(&asked.supplementary) {
        if decided_ids.contains(&entry.id) {
            continue;
        }
        if authored {
            retry.0.insert(entry.id.clone(), entry.clone());
            if let Some(notes) = feedback.get_mut(&entry.id) {
                notes.push(finding.clone());
            }
        } else {
            pending.remove(&entry.id);
            reused.remove(&entry.id);
            unproven.insert(entry.id.clone(), finding.clone());
        }
    }
    Ok(decided)
}

/// The checks of `ids` a passability judge that could not complete left
/// undecided, whose fault it is, and the finding each keeps.
pub(super) struct Held {
    pub(super) ids: BTreeSet<String>,
    pub(super) authored: bool,
    pub(super) finding: String,
}

/// What a passability judge that could not complete (`error`) holds of the
/// sound checks `ids`; `None` when `error` is no incomplete freeze.
pub(super) fn passability_failure(error: &anyhow::Error, ids: &BTreeSet<String>) -> Option<Held> {
    let incomplete = FreezeIncomplete::caused(error)?;
    // Only the checks the judge left without a verdict; every other sound
    // check keeps its own outcome.
    let affected: BTreeSet<String> = (incomplete.deferred().iter())
        .filter(|id| ids.contains(*id))
        .cloned()
        .collect();
    let authored = incomplete.is_authored_judge_reply();
    let finding = if authored {
        format!(
            "the passability judge returned no usable authored verdict: {error:#}; this check remains pending for judge retry"
        )
    } else {
        format!(
            "the passability judge could not complete for a provider or host fault: {error:#}; this check remains pending for the host and is judged again on resume"
        )
    };
    Some(Held {
        ids: if affected.is_empty() {
            ids.clone()
        } else {
            affected
        },
        authored,
        finding,
    })
}

/// Authored passability-judge replies stay pending per affected check.
#[cfg(test)]
pub(super) fn authored_passability_pending(
    error: &anyhow::Error,
    ids: &BTreeSet<String>,
) -> Option<(BTreeSet<String>, String)> {
    passability_failure(error, ids)
        .filter(|held| held.authored)
        .map(|held| (held.ids, held.finding))
}
