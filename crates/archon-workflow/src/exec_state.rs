use chrono::Utc;
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::executor::ExecutionReport;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::spec::StageSpec;

pub(crate) fn finish_run(run: &mut WorkflowRun) {
    if run.stages.values().all(|stage| stage.is_terminal()) {
        run.status = if run.stages.values().any(|s| s.status == StageStatus::Failed) {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        };
    }
    run.mark_updated();
}

pub(crate) fn pause_for_human_gate(
    run: &mut WorkflowRun,
    stage: &StageSpec,
    seq: &mut u64,
    log: &WorkflowEventLog,
) -> WorkflowResult<()> {
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.status = StageStatus::Paused;
    state.completed_at = None;
    run.status = RunStatus::Paused;
    run.mark_updated();
    log.emit(
        &run.id,
        *seq,
        WorkflowEventKind::Paused,
        json!({"stage": stage.id, "action": "human_gate", "status": "awaiting_approval"}),
    )?;
    *seq += 1;
    Ok(())
}

pub(crate) fn mark_started(run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.status = StageStatus::Running;
    state.attempt += 1;
    state.started_at = Some(Utc::now());
    state.error = None;
    run.mark_updated();
    Ok(())
}

pub(crate) fn mark_finished(
    run: &mut WorkflowRun,
    stage: &StageSpec,
    status: StageStatus,
    error: Option<String>,
) -> WorkflowResult<()> {
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.status = status;
    state.completed_at = Some(Utc::now());
    state.error = error;
    if status == StageStatus::Failed {
        run.status = RunStatus::Failed;
    }
    run.mark_updated();
    Ok(())
}

pub(crate) fn report(run: &WorkflowRun) -> ExecutionReport {
    ExecutionReport {
        run_id: run.id.clone(),
        completed: run
            .stages
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    StageStatus::Accepted | StageStatus::ForcedAccepted
                )
            })
            .count(),
        failed: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Failed)
            .count(),
        skipped: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Skipped)
            .count(),
    }
}
