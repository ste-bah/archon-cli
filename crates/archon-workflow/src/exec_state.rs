use chrono::Utc;
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::executor::ExecutionReport;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::spec::StageSpec;
use crate::stage::stage_ready;

pub(crate) fn finish_run(run: &mut WorkflowRun) {
    if run.stages.values().all(|stage| stage.is_terminal()) {
        run.status = if run.stages.values().any(stage_is_cancelled) {
            RunStatus::Cancelled
        } else if run.stages.values().any(stage_is_failed) {
            RunStatus::Failed
        } else if run.stages.values().any(stage_is_blocked) {
            RunStatus::Blocked
        } else if run.stages.values().any(stage_needs_review) {
            RunStatus::NeedsReview
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
            .filter(|s| s.status == StageStatus::Accepted)
            .count(),
        blocked: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Blocked)
            .count(),
        forced_accepted: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::ForcedAccepted)
            .count(),
        failed: run.stages.values().filter(|s| stage_is_failed(s)).count(),
        skipped: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Skipped)
            .count(),
    }
}

pub(crate) fn stalled_running_reason(run: &WorkflowRun) -> Option<String> {
    if !matches!(run.status, RunStatus::Running) {
        return None;
    }
    if run
        .stages
        .values()
        .any(|stage| stage.status == StageStatus::Running)
    {
        return None;
    }
    let pending = run
        .spec
        .stages
        .iter()
        .filter(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == StageStatus::Pending)
        })
        .collect::<Vec<_>>();
    if pending.is_empty() || pending.iter().any(|stage| stage_ready(run, stage)) {
        return None;
    }
    let blocked = pending
        .iter()
        .take(8)
        .map(|stage| blocked_stage_detail(run, stage))
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "workflow has pending stages but no runnable stage: {blocked}"
    ))
}

fn blocked_stage_detail(run: &WorkflowRun, stage: &StageSpec) -> String {
    let missing = stage
        .depends_on
        .iter()
        .filter(|dep| !run.dependency_satisfied_stage(dep))
        .map(|dep| {
            let status = run
                .stages
                .get(dep)
                .map(|state| format!("{:?}", state.status).to_ascii_lowercase())
                .unwrap_or_else(|| "missing".to_string());
            format!("{dep}={status}")
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        format!("{} is pending but not schedulable", stage.id)
    } else {
        format!("{} waits for {}", stage.id, missing.join(","))
    }
}

fn stage_is_failed(stage: &crate::run::StageState) -> bool {
    stage.status == StageStatus::Failed
}

fn stage_is_blocked(stage: &crate::run::StageState) -> bool {
    stage.status == StageStatus::Blocked
}

fn stage_needs_review(stage: &crate::run::StageState) -> bool {
    stage.status == StageStatus::NeedsReview
}

fn stage_is_cancelled(stage: &crate::run::StageState) -> bool {
    stage.status == StageStatus::Cancelled
}
