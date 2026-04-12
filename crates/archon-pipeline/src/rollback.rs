//! Rollback engine and failure policy dispatch.

use std::sync::Arc;

use crate::dag::Dag;
use crate::error::PipelineError;
use crate::run::{PipelineRun, PipelineState, StepRunState};
use crate::spec::OnFailurePolicy;
use crate::store::{AuditEvent, PipelineStateStore};

/// Action to take after a step fails and retries are exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    Rollback,
    Skip,
    Fail,
}

/// Maps the spec's `OnFailurePolicy` to a concrete `FailureAction`.
/// `Retry` falls through to `Rollback` since retry already happened.
pub fn handle_step_failure(policy: OnFailurePolicy) -> FailureAction {
    match policy {
        OnFailurePolicy::Retry | OnFailurePolicy::Rollback => FailureAction::Rollback,
        OnFailurePolicy::Skip => FailureAction::Skip,
        OnFailurePolicy::Fail => FailureAction::Fail,
    }
}

/// Orchestrates rollback of completed steps after a terminal failure.
pub struct RollbackEngine {
    store: Arc<PipelineStateStore>,
}

impl RollbackEngine {
    pub fn new(store: Arc<PipelineStateStore>) -> Self {
        Self { store }
    }

    /// Roll back a pipeline run: delete checkpoints for completed steps
    /// in reverse topological order, emit RolledBack audit events, then
    /// delete the run's state files (preserving audit.log).
    pub fn rollback(
        &self,
        run: &mut PipelineRun,
        dag: &Dag,
    ) -> Result<(), PipelineError> {
        // Collect finished step IDs in reverse topological order.
        let finished_steps: Vec<String> = dag
            .levels
            .iter()
            .flat_map(|level| level.iter())
            .filter(|step_id| {
                run.steps
                    .get(*step_id)
                    .map(|sr| sr.state == StepRunState::Finished)
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // Delete checkpoints and audit each rollback.
        for step_id in &finished_steps {
            self.store.delete_checkpoint(run.id, step_id)?;
            self.store.append_audit(
                run.id,
                &AuditEvent::RolledBack {
                    step: step_id.clone(),
                },
            )?;
        }

        // Transition to RolledBack state.
        run.state = PipelineState::RolledBack;
        run.finished_at = Some(chrono::Utc::now());
        self.store.save_state(run)?;

        // Delete state files (preserves audit.log).
        self.store.delete_run(run.id)?;

        Ok(())
    }
}
