//! One in-run retry of a write branch the host cut with work on disk.
//!
//! A branch that hits the host call timeout used to end the wave then and
//! there: its partial patch was captured, the stage went `needs_review`, every
//! dependent wave stalled behind it, and the operator was expected to run
//! `workflow resume` by hand — for work that, when captured, was an 88 KB patch
//! with every declared test already green. The resume machinery that applies a
//! captured patch to a fresh session was never invoked in-run. Now it is, once:
//! the branch is re-asked in the same worktree, told what it holds, under a
//! short budget. A second failure stalls exactly as before.
use super::*;

/// The transport row that makes the retry visible beside the `call_timeout`
/// row the host wrote for the session it follows.
pub(super) const RETRY_ROW_KIND: &str = "write_branch_timeout_retry";

/// The sentence the retry is told beyond the resume preamble.
pub(super) const RETRY_INSTRUCTION: &str = "The declared focused tests are believed to pass; run them once and return the result envelope.";

/// A branch outcome the host's timer produced, as opposed to a verdict on the
/// work or a resource the branch could not take.
pub(super) fn timed_out_with_work_unjudged(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("branch_runtime_timeout")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && result
            .data
            .get("branch_host_resource_contention")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
}

/// The budget the retry runs under: the smaller of the host's per-dispatch
/// timeout and the configured retry budget, or whichever exists.
pub(super) fn retry_budget(dispatch: &dyn WorkflowAgentDispatch) -> Option<std::time::Duration> {
    super::partial_work::effective_call_budget(
        dispatch.dispatch_timeout(),
        dispatch.timeout_retry_budget(),
        std::time::Duration::ZERO,
    )
}

/// The branch execution for the retry: the same call, its task re-rendered
/// with the resume preamble over the captured partial and what the cut
/// session was refused and last ran, and the per-dispatch timeout pinned to
/// the retry budget.
pub(super) fn retry_execution(
    branch: &WorktreeBranchExecution,
    task: &str,
    partial: &super::partial_work::PartialWork,
    budget: Option<std::time::Duration>,
    memory: &super::session_memory::SessionMemory,
) -> WorktreeBranchExecution {
    let mut execution = branch.execution.clone();
    execution.call.options.task = Some(super::partial_work::with_host_preamble(
        &format!("{RETRY_INSTRUCTION}\n\n{task}"),
        budget,
        Some(partial),
        memory,
    ));
    if let Some(budget) = budget {
        execution.call.options.extra.insert(
            crate::agent_dispatch_port::DISPATCH_TIMEOUT_OVERRIDE_KEY.to_string(),
            serde_json::Value::from(budget.as_secs()),
        );
    }
    WorktreeBranchExecution {
        id: branch.id.clone(),
        role: branch.role.clone(),
        input_hash: branch.input_hash.clone(),
        workspace_root: branch.workspace_root.clone(),
        execution,
        // The retry prompt already says what the worktree holds; a transport
        // drop inside the retry re-sends it as is.
        refresh: None,
        // The retry's loop is bounded by the retry's budget, not by the first
        // session's call budget: a transport drop inside the retry may re-ask,
        // but never past the minutes the retry was given.
        time_budget: BranchTimeBudget::Fixed(budget),
    }
}

/// Which outcome the branch reports after the retry: an accepted retry stands
/// on its own; anything else keeps the first session's timeout — its
/// `write_branch_timeout_*` gap is what the stall path and `resume` key on —
/// with the retry's verdict recorded beside it.
pub(super) fn settle(first: WorkflowV2Result, retry: WorkflowV2Result) -> WorkflowV2Result {
    if matches!(
        retry.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return retry;
    }
    let mut result = first;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "in-run retry with partial work applied ended {:?}: {}",
            retry.status, retry.summary
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "timeout_retry".to_string(),
            serde_json::json!({"status": retry.status, "summary": retry.summary}),
        );
    }
    result
}

/// Append one row to the stage's `transport.jsonl`, the file the host's
/// `call_timeout` row for the first session went to. Best effort: a row that
/// cannot be written must not fail the retry it describes.
pub(super) fn record_retry_row(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    partial: &super::partial_work::PartialWork,
) {
    let row = serde_json::json!({
        "kind": RETRY_ROW_KIND,
        "call_id": call_id,
        "item_id": item_id,
        "patch_files": partial.files.len(),
        "patch_bytes": partial.bytes,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
    });
    let path = v2_store.root().join("transport.jsonl");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "{row}")
        });
}
