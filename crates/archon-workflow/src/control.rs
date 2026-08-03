use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunControlDecision {
    Continue,
    Paused { generation: u64 },
    Cancelled { generation: u64 },
}

#[derive(Debug, Clone)]
pub struct RunControl {
    store: WorkflowStore,
    run_id: String,
}

impl RunControl {
    pub fn new(store: WorkflowStore, run_id: impl Into<String>) -> Self {
        Self {
            store,
            run_id: run_id.into(),
        }
    }

    pub fn poll(&self) -> WorkflowResult<RunControlDecision> {
        decision_from_run(&self.store.load_state(&self.run_id)?)
    }

    pub fn checkpoint(&self, local: &mut WorkflowRun) -> WorkflowResult<RunControlDecision> {
        let current = self.store.load_state(&self.run_id)?;
        let decision = decision_from_run(&current)?;
        match decision {
            RunControlDecision::Continue => {
                if current.generation > local.generation {
                    local.generation = current.generation;
                }
            }
            RunControlDecision::Paused { generation } => {
                local.status = RunStatus::Paused;
                local.generation = generation;
                reopen_running_stages(local, StageStatus::Paused);
            }
            RunControlDecision::Cancelled { generation } => {
                local.status = RunStatus::Cancelled;
                local.generation = generation;
                cancel_running_stages(local);
            }
        }
        Ok(decision)
    }
}

/// Checkpoint a V2 host call against the run's pause/cancel control.
///
/// Called immediately before and after every branch dispatch, so a pause or
/// cancel written while a fan-out is in flight is observed at the next branch
/// boundary rather than at the end of the wave. The observed decision is
/// persisted with `save_state_preserving_control` before the error is raised,
/// so the stopped run's state reflects why it stopped even though the caller
/// unwinds.
pub fn poll_v2_run_control(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
) -> WorkflowResult<()> {
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

fn decision_from_run(run: &WorkflowRun) -> WorkflowResult<RunControlDecision> {
    Ok(match run.status {
        RunStatus::Paused => RunControlDecision::Paused {
            generation: run.generation,
        },
        RunStatus::Cancelled => RunControlDecision::Cancelled {
            generation: run.generation,
        },
        _ => RunControlDecision::Continue,
    })
}

fn reopen_running_stages(run: &mut WorkflowRun, status: StageStatus) {
    for stage in run.stages.values_mut() {
        if stage.status == StageStatus::Running {
            stage.status = status;
            stage.completed_at = None;
        }
    }
    run.mark_updated();
}

fn cancel_running_stages(run: &mut WorkflowRun) {
    for stage in run.stages.values_mut() {
        if matches!(stage.status, StageStatus::Running | StageStatus::Pending) {
            stage.status = StageStatus::Cancelled;
            stage.completed_at.get_or_insert_with(chrono::Utc::now);
            stage.error.get_or_insert_with(|| "cancelled".to_string());
        }
    }
    for item in run.items.values_mut() {
        if matches!(item.status, StageStatus::Running | StageStatus::Pending) {
            item.status = StageStatus::Cancelled;
            item.error.get_or_insert_with(|| "cancelled".to_string());
        }
    }
    run.mark_updated();
}
