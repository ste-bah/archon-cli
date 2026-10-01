//! The read-only fan-out's branch timeout and branch events, split from
//! `workflow_live_v2_read_only_b` to keep it within the source ceiling.

use super::*;

/// The branch timeout and the name of the setting it came from.
pub(super) fn read_only_branch_timeout_secs(
    call_id: &str,
    review_map: bool,
    config: &GeneratedWorkflowConfig,
) -> (u64, &'static str) {
    // REM-16: a review map branch runs commands too, under the same bound.
    if review_map
        || call_id.starts_with("verification-wave-")
        || call_id.starts_with("review-verification-wave-")
    {
        return (
            u64::from(config.verification_branch_timeout_secs),
            "verification_branch_timeout_secs",
        );
    }
    (
        u64::from(config.host_call_timeout_secs),
        "host_call_timeout_secs",
    )
}

pub(super) fn branch_event_label(outcome: &WorkflowV2BranchOutcome) -> &'static str {
    // First: an inactivity cut also travels inside the host-cut wrapper, and
    // the record must name the bound that fired.
    if outcome
        .error
        .as_deref()
        .is_some_and(archon_workflow::error::is_inactivity_timeout_text)
    {
        return "branch_inactive";
    }
    if outcome
        .error
        .as_deref()
        .is_some_and(|error| error.to_ascii_lowercase().contains("timed out"))
    {
        return "branch_timed_out";
    }
    if outcome.status == WorkflowV2Status::Cancelled {
        return "branch_cancelled";
    }
    if outcome.status == WorkflowV2Status::Failed {
        return "branch_failed";
    }
    "branch_finished"
}

pub(super) fn emit_v2_branch_event(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) {
    let Ok(seq) = store.next_event_seq(run_id) else {
        return;
    };
    let _ = WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail);
}
