//! Which stored branch outcomes a restarted fan-out may reuse instead of
//! re-running.
//!
//! A fan-out that restarts re-derives the same items, so every item whose
//! previous outcome is still trustworthy should be reused rather than paid for
//! again. Trustworthy is deliberately narrow: the outcome must be terminal-good
//! (`Accepted`/`Noop`), carry a result that agrees with the outcome status and
//! passes its own validation, and record the input hash it was produced from —
//! which must still match the item as it is derived today. An item whose input
//! changed is not the same item.
//!
//! "Its input" is the item as AUTHORED — see [`crate::v2::reuse_identity`].
//! The host stamps current-tree data (line budgets, repository root, a
//! discovered scope) into the input before asking here, and hashing that made
//! every earlier wave's identity move whenever a later wave touched a shared
//! file (Issue-24). Both the save sites and this comparison now read the
//! authored identity, and a hash stored before it existed is still honoured.
//!
//! One branch is reusable whatever its hash or its current record say: one
//! whose patch the host applied, for tasks this run has recorded landed. It
//! is reused AS the record that landed it — a replay's no-op or needs-review
//! record is not what a committed task looks like — and that record is
//! re-saved as the current one. A committed task has nothing left to write,
//! and re-dispatching it is the failure this module exists to prevent — see
//! [`landed_for_this_run`] and [`landing_record`].
//!
//! Wave call ids additionally require durable completion evidence, because
//! their outcomes feed the completion ledger; an outcome with no evidence would
//! be reused into a credit it cannot support.
//!
//! # Reuse is keyed on `(call_id, item_id)`, and that is deliberate
//!
//! It is tempting to widen this to a call-id-independent identity, because a
//! retried wave gets a NEW call id (`remediation-wave-1` →
//! `remediation-wave-1-1`, minted in `lifecycle_driver::implementation`) and new
//! branch ids (`{call_id}-{item_id}`, minted in `call_data::source`), so nothing
//! from the previous attempt is visible to the next one.
//!
//! Widening it would be WRONG, because of what a new call id means here. The
//! retry wave is not the same wave run again: its items come from a follow-up
//! inventory the driver derives from `non_accepted_outcomes` of the previous
//! wave and filters through `enforce_outcome_repair_accounting`. Membership in a
//! retry wave therefore MEANS "this did not resolve". An item that resolved is
//! already credited (`matching_accepted_ids`) and is never rescheduled, so there
//! is no accepted sibling to rescue; and anything that IS rescheduled is
//! something the accounting has just decided must be redone.
//!
//! The decisive case is the review loop. `review-remediation-wave-{n+1}` can
//! legitimately ask for the SAME remediation as round `n` — the follow-up
//! inventory is even hydrated from the previous round's source items — and that
//! repetition is the signal that round `n`'s accepted fix did not stick. A
//! payload-identity cache would answer it with round `n`'s accepted outcome,
//! skip the work, and let the loop declare convergence it never reached. A
//! repeated execution costs money; a wrong reuse costs correctness, silently.
//!
//! The accepted-siblings loss that motivated this note came from
//! `write::worktree_wave`, where one branch's `Err` aborted the collection
//! before any sibling was persisted. That is fixed at the source, in
//! `worktree_wave_outcomes`. See `cross_attempt_reuse_is_refused` below, which
//! pins this decision.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::WorkflowResult;
use crate::generated_contract::canonical_task_ids_from_generated_value;
use crate::v2::result::WorkflowV2Status;
use crate::v2::result_store::WorkflowV2ResultStore;
use crate::v2::reuse_identity::recorded_hash_matches;
use crate::v2::scheduler::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem};
use crate::v2::write::{landed_task_ids, manifest_path_for};
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// Split `items` into the outcomes that may be reused and the items that must
/// still run.
pub fn split_reusable_branch_outcomes(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    items: Vec<WorkflowV2FanoutItem>,
) -> WorkflowResult<(Vec<WorkflowV2BranchOutcome>, Vec<WorkflowV2FanoutItem>)> {
    let audit = crate::repository_audit::reuse::load_state(v2_store)?;
    // The dependency gate's own landed-task source (TD-058): every accepted or
    // no-op outcome recorded in this run, whatever call produced it — current
    // AND superseded, because after a few resumes the current record for a
    // landed task is the replay's no-op and the accepted record that landed
    // it lives under `superseded/`.
    let mut outcomes = v2_store.load_branch_outcomes()?;
    outcomes.extend(v2_store.load_superseded_branch_outcomes());
    let landed = landed_task_ids(&outcomes);
    let mut reused = Vec::new();
    let mut pending = Vec::new();
    for item in items {
        if item.call.write_mode.is_some()
            && let Some(state) = &audit
            && !crate::repository_audit::reuse::admits(state, &item.call.options.target_files)?
        {
            pending.push(item);
            continue;
        }
        let current = v2_store.load_branch_outcome(call_id, &item.id)?;
        // A landed branch is reused as its landing record FIRST, ahead of the
        // hash match: a replay's no-op or needs-review record can carry the
        // same authored identity, and reusing it would read downstream as
        // "not implemented" for a task this run committed.
        if landed_for_this_run(v2_store, call_id, &item, &landed)
            && let Some(landing) = landing_record(v2_store, call_id, &item.id, current.as_ref())
            && reusable_branch_outcome(&landing)
            && (!completion_evidence_call_id(call_id) || !landing.completion_evidence.is_empty())
        {
            if current.as_ref() != Some(&landing) {
                v2_store.save_branch_outcome(call_id, &landing)?;
            }
            reused.push(landing);
            continue;
        }
        match current {
            Some(outcome) if reusable_branch_outcome_for_item(call_id, &outcome, &item) => {
                reused.push(outcome)
            }
            _ => pending.push(item),
        }
    }
    Ok((reused, pending))
}

/// Whether an outcome is terminal-good, self-consistent, and hash-stamped.
///
/// `failure_kind` must also be absent. It is not redundant with the status
/// check: `write::save_write_branch_outcome` derives it from
/// `failure_kind_from_write_result`, which reads `result.data["failure_kind"]`
/// FIRST and only falls back to the status. `data` on an accepted branch is the
/// AGENT's, so an agent returning `status: accepted` alongside
/// `data.failure_kind` produces a stored outcome that says both "this
/// succeeded" and "this failed". Reuse takes the pessimistic reading and
/// re-runs, because the cost of being wrong is asymmetric: a needless execution
/// versus crediting failed work as done.
pub fn reusable_branch_outcome(outcome: &WorkflowV2BranchOutcome) -> bool {
    matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && outcome.failure_kind.is_none()
        && outcome
            .result
            .as_ref()
            .is_some_and(|result| result.status == outcome.status && result.validate().is_ok())
        && outcome.item_input_hash.is_some()
}

fn reusable_branch_outcome_for_item(
    call_id: &str,
    outcome: &WorkflowV2BranchOutcome,
    item: &WorkflowV2FanoutItem,
) -> bool {
    reusable_branch_outcome(outcome)
        && (!completion_evidence_call_id(call_id) || !outcome.completion_evidence.is_empty())
        && outcome
            .item_input_hash
            .as_deref()
            .is_some_and(|recorded| recorded_hash_matches(recorded, item))
}

/// Whether this run has landed the item's tasks through this branch, whatever
/// the item's hash or the current record say now. Same `call_id` only: the
/// review-loop rule in this module's header is untouched, because a retry
/// wave never sees another call's outcome.
///
/// Two things must both hold. Every canonical task the item claims now is
/// among `landed` (so an item re-authored to own a task nothing landed is not
/// credited with it), and the host has a receipt that THIS branch's patch
/// landed: the manifest `apply_wave` marked `applied`, or an earlier record
/// for the same branch that was accepted with `patch_landed` — the current
/// record is often a replay's no-op or needs-review, with the accepted one
/// superseded.
///
/// "Applied" is the host's receipt, not the branch's claim. `patch_landed` is
/// stamped when the worktree's patch is CAPTURED (`write::mark_patch_landed`)
/// and the outcome is saved before the wave is applied; the manifest is what
/// `apply_wave` writes to. Serial and coordinated writes carry neither and
/// never take this path.
fn landed_for_this_run(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item: &WorkflowV2FanoutItem,
    landed: &[String],
) -> bool {
    let source = item.input.get("item").unwrap_or(&item.input);
    let claimed = canonical_task_ids_from_generated_value(source, None);
    if claimed.is_empty() || !claimed.iter().all(|id| landed.contains(id)) {
        return false;
    }
    manifest_applied(v2_store, call_id, &item.id)
        || superseded_records_for(v2_store, call_id, &item.id)
            .iter()
            .any(accepted_with_patch_landed)
}

/// The record that landed this branch: the most recent outcome for
/// `(call_id, item_id)` — current or superseded — that is accepted with
/// `patch_landed`. That is what a landed task is reused AS: a replay's no-op
/// or needs-review record, even reused, reads downstream as "not
/// implemented", and a replay that blew the line cap must not un-land a task
/// its own earlier attempt committed. The caller re-saves it as the current
/// record so the run directory says the same.
fn landing_record(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    current: Option<&WorkflowV2BranchOutcome>,
) -> Option<WorkflowV2BranchOutcome> {
    current
        .filter(|outcome| accepted_with_patch_landed(outcome))
        .cloned()
        .or_else(|| {
            superseded_records_for(v2_store, call_id, item_id)
                .into_iter()
                .find(accepted_with_patch_landed)
        })
}

fn accepted_with_patch_landed(outcome: &WorkflowV2BranchOutcome) -> bool {
    outcome.status == WorkflowV2Status::Accepted
        && outcome
            .result
            .as_ref()
            .and_then(|result| result.data.get("patch_landed"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

/// Every record a later save replaced for this branch, newest first, from the
/// call's `superseded/` directory (`result_store::archive_superseded_json`,
/// which renames and so keeps each record's own write time). An unreadable
/// archived file is skipped: the archive is history.
fn superseded_records_for(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
) -> Vec<WorkflowV2BranchOutcome> {
    let current = v2_store.branch_outcome_path(call_id, item_id);
    let Some(dir) = current.parent().map(|parent| parent.join("superseded")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut records: Vec<(std::time::SystemTime, WorkflowV2BranchOutcome)> = entries
        .flatten()
        .filter_map(|entry| {
            let written = entry.metadata().and_then(|meta| meta.modified()).ok()?;
            let bytes = std::fs::read(entry.path()).ok()?;
            let outcome = serde_json::from_slice::<WorkflowV2BranchOutcome>(&bytes).ok()?;
            (outcome.item_id == item_id).then_some((written, outcome))
        })
        .collect();
    records.sort_by_key(|(written, _)| std::cmp::Reverse(*written));
    records.into_iter().map(|(_, outcome)| outcome).collect()
}

/// Whether the persisted patch manifest for this branch says `applied`.
///
/// `run_root` is derived exactly as `write::worktree_fanout_setup` derives it:
/// the parent of the v2 store root.
fn manifest_applied(v2_store: &WorkflowV2ResultStore, call_id: &str, item_id: &str) -> bool {
    let run_root = v2_store
        .root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| v2_store.root().to_path_buf());
    let path = manifest_path_for(&run_root, call_id, item_id);
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PatchManifest>(&bytes).ok())
        .is_some_and(|manifest| manifest.status == ManifestStatus::Applied)
}

fn completion_evidence_call_id(call_id: &str) -> bool {
    call_id.starts_with("noop-proof-verification-")
        || call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
        || call_id.starts_with("verification-wave-")
        || call_id.starts_with("review-verification-wave-")
}

/// Positional index of every item, so reused and freshly run outcomes can be
/// restored to the order the fan-out derived them in.
pub fn branch_item_order(items: &[WorkflowV2FanoutItem]) -> BTreeMap<String, usize> {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| (item.id.clone(), idx))
        .collect()
}

/// Restore `outcomes` to the order recorded by [`branch_item_order`].
pub fn sort_branch_outcomes_by_order(
    outcomes: &mut [WorkflowV2BranchOutcome],
    order: &BTreeMap<String, usize>,
) {
    outcomes.sort_by_key(|outcome| order.get(&outcome.item_id).copied().unwrap_or(usize::MAX));
}

#[cfg(test)]
#[path = "branch_cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "branch_cache_landed_tests.rs"]
mod landed_tests;
