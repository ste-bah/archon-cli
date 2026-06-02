use chrono::Utc;

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    Resume,
    Pause,
    Cancel,
    RestartStage(String),
}

#[derive(Debug, Clone)]
pub struct LifecycleController {
    store: WorkflowStore,
}

impl LifecycleController {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }

    pub fn apply(&self, run_id: &str, action: LifecycleAction) -> WorkflowResult<WorkflowRun> {
        let mut run = self.store.load_state(run_id)?;
        match action {
            LifecycleAction::Resume => run.status = RunStatus::Running,
            LifecycleAction::Pause => run.status = RunStatus::Paused,
            LifecycleAction::Cancel => run.status = RunStatus::Cancelled,
            LifecycleAction::RestartStage(stage_id) => rewind_stage(&mut run, &stage_id)?,
        }
        run.updated_at = Utc::now();
        self.store.save_state(&run)?;
        Ok(run)
    }
}

fn rewind_stage(run: &mut WorkflowRun, stage_id: &str) -> WorkflowResult<()> {
    let state = run
        .stages
        .get_mut(stage_id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("unknown stage {stage_id}")))?;
    state.status = StageStatus::Pending;
    state.error = None;
    state.completed_at = None;
    state.artifacts.clear();
    run.status = RunStatus::Running;
    Ok(())
}
