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

use crate::error::WorkflowResult;
use crate::v2::result::WorkflowV2Status;
use crate::v2::result_store::WorkflowV2ResultStore;
use crate::v2::scheduler::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem};

/// Split `items` into the outcomes that may be reused and the items that must
/// still run.
pub fn split_reusable_branch_outcomes(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    items: Vec<WorkflowV2FanoutItem>,
) -> WorkflowResult<(Vec<WorkflowV2BranchOutcome>, Vec<WorkflowV2FanoutItem>)> {
    let mut reused = Vec::new();
    let mut pending = Vec::new();
    for item in items {
        match v2_store.load_branch_outcome(call_id, &item.id)? {
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
            .is_some_and(|recorded| recorded == item.input_hash())
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
