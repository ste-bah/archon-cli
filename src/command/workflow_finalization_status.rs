//! Sanitized workflow finalization and run-end observer status.

use anyhow::{Context, Result};
use archon_workflow::{FinalizationRecordV1, RunEndObserverStateV1, WorkflowStore};

const FINALIZATION_RECORD_PATH: &str = "v2/finalization.json";

pub(super) fn render(store: &WorkflowStore, run_id: &str) -> Result<Option<String>> {
    let path = store.run_dir(run_id).join(FINALIZATION_RECORD_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let record: FinalizationRecordV1 = serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("reading finalization status {}", path.display()))?,
    )
    .with_context(|| format!("parsing finalization status {}", path.display()))?;
    let mut out = String::from("\nfinalization:\n");
    out.push_str(&format!(
        "run_kind: {}\nterminal_status: {}\nterminal_v2_status: {}\nterminal_state_committed: {}\nterminal_event_committed: {}\n",
        run_kind(record.run_kind),
        run_status(&record.terminal_status),
        record
            .terminal_v2_status
            .map(v2_status)
            .unwrap_or("none"),
        record.terminal_state_committed,
        record.terminal_event_committed,
    ));
    match record.observer_state {
        None => out.push_str("observer: none\n"),
        Some(RunEndObserverStateV1::Pending) => out.push_str("observer: pending\n"),
        Some(RunEndObserverStateV1::Failed { .. }) => out.push_str("observer: failed\n"),
        Some(RunEndObserverStateV1::Completed { outcome }) => out.push_str(&format!(
            "observer: completed authority=observe_only evaluated_floors={} policy_findings={} operational_deferrals={}\n",
            outcome.evaluated_floor_count,
            outcome.policy_finding_count,
            outcome.operational_deferral_count,
        )),
    }
    Ok(Some(out))
}

fn run_kind(kind: archon_workflow::WorkflowRunKind) -> &'static str {
    match kind {
        archon_workflow::WorkflowRunKind::AuthoredTaskWorkflow => "authored_task_workflow",
        archon_workflow::WorkflowRunKind::LegacyDecomposed => "legacy_decomposed",
        archon_workflow::WorkflowRunKind::FixedDecompositionV1 => "fixed_decomposition_v1",
        archon_workflow::WorkflowRunKind::FixedOrSavedScript => "fixed_or_saved_script",
    }
}

fn run_status(status: &archon_workflow::RunStatus) -> &'static str {
    match status {
        archon_workflow::RunStatus::Planned => "planned",
        archon_workflow::RunStatus::Running => "running",
        archon_workflow::RunStatus::Paused => "paused",
        archon_workflow::RunStatus::NeedsReview => "needs_review",
        archon_workflow::RunStatus::Blocked => "blocked",
        archon_workflow::RunStatus::Failed => "failed",
        archon_workflow::RunStatus::Cancelled => "cancelled",
        archon_workflow::RunStatus::Completed => "completed",
    }
}

fn v2_status(status: archon_workflow::WorkflowV2Status) -> &'static str {
    match status {
        archon_workflow::WorkflowV2Status::Pending => "pending",
        archon_workflow::WorkflowV2Status::Running => "running",
        archon_workflow::WorkflowV2Status::Accepted => "accepted",
        archon_workflow::WorkflowV2Status::Noop => "noop",
        archon_workflow::WorkflowV2Status::NeedsReview => "needs_review",
        archon_workflow::WorkflowV2Status::Blocked => "blocked",
        archon_workflow::WorkflowV2Status::Failed => "failed",
        archon_workflow::WorkflowV2Status::Cancelled => "cancelled",
    }
}
