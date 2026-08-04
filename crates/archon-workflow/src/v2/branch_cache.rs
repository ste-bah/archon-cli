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
pub fn reusable_branch_outcome(outcome: &WorkflowV2BranchOutcome) -> bool {
    matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && outcome
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
