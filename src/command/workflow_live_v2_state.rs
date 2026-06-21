use anyhow::Result;
use archon_workflow::{
    RunControl, RunControlDecision, RunStatus, StageStatus, WorkflowError, WorkflowStore,
    WorkflowV2HostCall, WorkflowV2ResultStore, WorkflowV2Status,
};

pub(super) fn poll_v2_run_control(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
) -> archon_workflow::WorkflowResult<()> {
    let mut local = store.load_state(run_id)?;
    match RunControl::new(store.clone(), run_id).checkpoint(&mut local)? {
        RunControlDecision::Continue => Ok(()),
        RunControlDecision::Paused { generation } => {
            store.save_state_preserving_control(&local)?;
            Err(WorkflowError::ControlPaused(format!(
                "generation {generation} observed before/after V2 call '{}'",
                call_id
            )))
        }
        RunControlDecision::Cancelled { generation } => {
            store.save_state_preserving_control(&local)?;
            Err(WorkflowError::ControlCancelled(format!(
                "generation {generation} observed before/after V2 call '{}'",
                call_id
            )))
        }
    }
}

pub(super) fn mark_v2_call_running(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
) -> archon_workflow::WorkflowResult<()> {
    let mut run = store.load_state(run_id)?;
    if matches!(run.status, RunStatus::Paused | RunStatus::Cancelled) {
        return Ok(());
    }
    run.status = RunStatus::Running;
    if let Some(stage) = run.stage_mut(call_id) {
        stage.status = StageStatus::Running;
        stage.error = None;
        stage.started_at.get_or_insert_with(chrono::Utc::now);
        stage.completed_at = None;
    }
    run.mark_updated();
    store.save_state_preserving_control(&run)
}

pub(super) fn sync_v2_summary_to_run(
    store: &WorkflowStore,
    run_id: &str,
    calls: &[WorkflowV2HostCall],
    v2_store: &WorkflowV2ResultStore,
    status: WorkflowV2Status,
) -> Result<()> {
    let mut run = store.load_state(run_id)?;
    for call in calls {
        if let Some(stage) = run.stage_mut(&call.id) {
            let call_status = v2_store
                .load_call_record(&call.id)?
                .map(|record| record.status)
                .unwrap_or(WorkflowV2Status::Pending);
            stage.status = stage_status_from_v2(call_status);
            if matches!(
                stage.status,
                StageStatus::Accepted
                    | StageStatus::Blocked
                    | StageStatus::NeedsReview
                    | StageStatus::Failed
                    | StageStatus::Cancelled
            ) {
                stage.completed_at = Some(chrono::Utc::now());
            }
        }
    }
    run.status = run_status_from_v2(status);
    run.mark_updated();
    store.save_state(&run)?;
    Ok(())
}

fn stage_status_from_v2(status: WorkflowV2Status) -> StageStatus {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => StageStatus::Accepted,
        WorkflowV2Status::Blocked => StageStatus::Blocked,
        WorkflowV2Status::NeedsReview => StageStatus::NeedsReview,
        WorkflowV2Status::Failed => StageStatus::Failed,
        WorkflowV2Status::Cancelled => StageStatus::Cancelled,
        WorkflowV2Status::Pending => StageStatus::Pending,
        WorkflowV2Status::Running => StageStatus::Running,
    }
}

fn run_status_from_v2(status: WorkflowV2Status) -> RunStatus {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => RunStatus::Completed,
        WorkflowV2Status::Blocked => RunStatus::Blocked,
        WorkflowV2Status::NeedsReview => RunStatus::NeedsReview,
        WorkflowV2Status::Failed => RunStatus::Failed,
        WorkflowV2Status::Cancelled => RunStatus::Cancelled,
        WorkflowV2Status::Pending | WorkflowV2Status::Running => RunStatus::Running,
    }
}
