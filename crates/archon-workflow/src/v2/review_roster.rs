//! The roster a review reduce is handed: every branch its source maps ran,
//! with the task each reviewed and how many findings it returned.
//!
//! Obs-22, run wf-719ff3b0. The adversarial reduce received only the map
//! findings. Two map branches (`adversarial-review-map-4` = TASK-DL-005,
//! `-8` = TASK-DL-009) had run fully and returned ZERO findings, so nothing
//! in the reducer's input mentioned those tasks, and it reported a HIGH
//! finding that they "have no adversarial review". False, and it would have
//! driven a remediation round for work nobody had faulted.
//!
//! A findings list cannot carry the absence of findings. The roster can: it
//! is built by the host from the STORED branch outcomes of each source map --
//! not from the findings -- so a branch that ran and reported nothing is on it
//! with `finding_count: 0`, and the reduce prompt says what that means. The
//! script's own best-effort roster (`reviewMapReduce`) is replaced by this one
//! whenever the host can build one: the host's records are the authority.
//!
//! Domain-agnostic: item ids, task ids, statuses and counts, nothing read from
//! inside a finding.

use serde::Serialize;
use serde_json::Value;

use super::call_execution::WorkflowV2CallExecution;
use super::outcome_envelope::outcomes_of;
use super::result_store::WorkflowV2ResultStore;
use super::review_findings::{collect_findings, review_contract, source_map_call_ids, task_ids_of};
use super::{WorkflowV2BranchOutcome, WorkflowV2Status};
use crate::WorkflowResult;
use crate::v2::completion_evidence::canonical_task_ids_from_result;

/// The reduce input field the roster travels under.
pub const BRANCH_ROSTER_KEY: &str = "branch_roster";

/// What the roster means, told to the critic beside it. One sentence, in the
/// prompt's typed constraints, so it cannot be lost in an authored task text.
///
/// A roster entry whose status is failed, blocked or cancelled did NOT review
/// its task. The rule used to say every entry was reviewed, which told the
/// reducer to read a failed branch's zero as a clean review; the host now
/// carries an `unreviewed` finding for such a task (`review_unreviewed`), and
/// the rule says so.
pub const BRANCH_ROSTER_RULE: &str = "An item in branch_roster with status accepted, noop or needs_review was reviewed, and a zero finding_count means reviewed with nothing to report \u{2014} never report such a task as unreviewed. An item with status failed, blocked or cancelled was NOT reviewed: the host has already recorded an unreviewed finding for its task, so do not restate it and never treat that task as reviewed clean.";

/// One branch of a source map, as the reduce sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchRosterEntry {
    pub item_id: String,
    pub canonical_task_ids: Vec<String>,
    pub status: WorkflowV2Status,
    pub finding_count: usize,
}

/// A roster entry for one stored branch outcome.
///
/// Task ids come from the outcome's `data.canonical_task_ids` -- stamped by
/// the host from the item input for every read-only branch since Obs-22 --
/// then from the wider result view (accepted task coverage) for records saved
/// before that stamp existed. A failed branch with no result keeps an empty
/// id list and a zero count: it IS on the roster, with its status, which is
/// the honest thing to show a reducer.
pub fn roster_entry(outcome: &WorkflowV2BranchOutcome) -> BranchRosterEntry {
    let (canonical_task_ids, finding_count) = match outcome.result.as_ref() {
        Some(result) => {
            let mut ids = task_ids_of(&result.data);
            if ids.is_empty() {
                ids = canonical_task_ids_from_result(result);
            }
            (ids, collect_findings(&result.data).len())
        }
        None => (Vec::new(), 0),
    };
    BranchRosterEntry {
        item_id: outcome.item_id.clone(),
        canonical_task_ids,
        status: outcome.status,
        finding_count,
    }
}

/// The roster of every branch the named source maps ran, in source order.
///
/// Read from the per-branch records first. A store that holds no branch file
/// for a call falls back to the outcome views the call record itself
/// carries, which are the same outcomes serialised (`fanout_outcome_views`).
/// A source with neither contributes nothing; `review_findings` already
/// reports a missing source, and the roster must not invent a branch.
pub fn roster_for_sources(
    store: &WorkflowV2ResultStore,
    sources: &[String],
) -> WorkflowResult<Vec<BranchRosterEntry>> {
    let mut roster = Vec::new();
    for call_id in sources {
        let mut outcomes = store.load_branch_outcomes_for_call(call_id)?;
        if outcomes.is_empty()
            && let Some(record) = store.load_call_record(call_id)?
            && is_fanout_record(&record.result.data)
        {
            outcomes = outcomes_of(&record.result.data)
                .into_iter()
                .filter_map(|view| serde_json::from_value::<WorkflowV2BranchOutcome>(view).ok())
                .collect();
        }
        roster.extend(outcomes.iter().map(roster_entry));
    }
    Ok(roster)
}

/// Only a fan-out record carries branch views. `outcomes_of` returns any
/// other value as one outcome of itself, and a `reduce_final` may name a
/// chunk reducer as its source; that record's `data` is not a branch.
fn is_fanout_record(data: &Value) -> bool {
    ["outcomes", "items"]
        .iter()
        .any(|key| data.get(key).is_some_and(Value::is_array))
}

/// The source map calls a review reduce names, if this call is one.
pub fn reduce_review_sources(execution: &WorkflowV2CallExecution) -> Vec<String> {
    review_contract(execution)
        .map(source_map_call_ids)
        .unwrap_or_default()
}

/// Whether a call's input already carries a roster the host built or the
/// script supplied.
pub fn carries_branch_roster(input: &Value) -> bool {
    input
        .get(BRANCH_ROSTER_KEY)
        .and_then(Value::as_array)
        .is_some_and(|roster| !roster.is_empty())
}

/// Put the host-built roster on a review reduce's input.
///
/// Runs at dispatch, after the reuse hash was taken from the script's own
/// input, so adding the roster never invalidates a recorded reduce -- the same
/// contract `source_data` resolution keeps. A non-empty host roster replaces
/// whatever the script sent under the key; an empty one (no source records at
/// all) leaves the script's, if any, alone. Calls without a review contract
/// naming source maps are untouched.
pub fn attach_branch_roster(
    execution: &mut WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<()> {
    let sources = reduce_review_sources(execution);
    if sources.is_empty() {
        return Ok(());
    }
    let roster = roster_for_sources(store, &sources)?;
    if roster.is_empty() {
        return Ok(());
    }
    let roster = serde_json::to_value(&roster)?;
    match &mut execution.input {
        Value::Object(object) => {
            object.insert(BRANCH_ROSTER_KEY.to_string(), roster);
        }
        other => {
            // Mirrors `execution_with_resolved_source`: a non-object input is
            // kept whole under `input` beside the host's additions.
            *other = serde_json::json!({ "input": other.take(), BRANCH_ROSTER_KEY: roster });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "review_roster_tests.rs"]
mod tests;
