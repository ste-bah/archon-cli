use crate::error::WorkflowResult;
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
