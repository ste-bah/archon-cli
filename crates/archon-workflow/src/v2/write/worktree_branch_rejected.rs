//! Recording a worktree branch result the adapter refused.
//!
//! Split from `worktree_branch_a.rs` for the 500-line ceiling.

use super::super::*;

pub(crate) fn persist_rejected_worktree_result(
    store: &WorkflowV2ResultStore,
    branch_id: &str,
    attempt: &str,
    result: &WorkflowV2Result,
    error: &str,
) {
    let raw_body = serde_json::to_string(result).unwrap_or_else(|_| result.summary.clone());
    let record = WorkflowV2RejectedOutput {
        attempt: attempt.to_string(),
        error: error.to_string(),
        raw_body,
    };
    let _ = store.append_rejected_output(branch_id, record);
}
