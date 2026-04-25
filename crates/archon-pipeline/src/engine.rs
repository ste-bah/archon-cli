//! Public pipeline engine API -- trait definition and default implementation.
//!
//! Provides the [`PipelineEngine`] trait for pipeline lifecycle operations
//! (parse, validate, run, status, cancel, resume) and a
//! [`DefaultPipelineEngine`] that wires together the parser, executor, and
//! state store.

use std::sync::Arc;

use async_trait::async_trait;

use crate::dag::{Dag, DagBuilder};
use crate::error::PipelineError;
use crate::executor::PipelineExecutor;
use crate::id::PipelineId;
use crate::parser::PipelineParser;
use crate::run::{PipelineRun, PipelineState, StepRunState};
use crate::spec::{PipelineFormat, PipelineSpec};
use crate::store::{AuditEvent, PipelineStateStore};
use crate::vars;

/// Public API for pipeline lifecycle operations.
#[async_trait]
pub trait PipelineEngine: Send + Sync {
    /// Parse a pipeline spec from a string.
    fn parse(&self, src: &str, fmt: PipelineFormat) -> Result<PipelineSpec, PipelineError>;

    /// Validate a parsed spec (DAG + variable refs).
    fn validate(&self, spec: &PipelineSpec) -> Result<Dag, PipelineError>;

    /// Execute a pipeline to completion.
    async fn run(&self, spec: PipelineSpec) -> Result<PipelineId, PipelineError>;

    /// Return current status of a pipeline run.
    async fn status(&self, id: PipelineId) -> Result<PipelineRun, PipelineError>;

    /// Cancel a running pipeline.
    async fn cancel(&self, id: PipelineId) -> Result<(), PipelineError>;

    /// Resume a failed/cancelled pipeline from last checkpoint.
    async fn resume(&self, id: PipelineId) -> Result<PipelineId, PipelineError>;

    /// List all pipeline run IDs.
    async fn list(&self) -> Result<Vec<crate::id::PipelineId>, PipelineError>;
}

/// Default implementation wiring parser, executor, and state store.
pub struct DefaultPipelineEngine {
    executor: Arc<PipelineExecutor>,
    store: Arc<PipelineStateStore>,
}

impl DefaultPipelineEngine {
    pub fn new(
        store: Arc<PipelineStateStore>,
        task_service: Arc<dyn archon_core::tasks::TaskService>,
    ) -> Self {
        let executor = Arc::new(PipelineExecutor::new(store.clone(), task_service));
        Self { executor, store }
    }
}

#[async_trait]
impl PipelineEngine for DefaultPipelineEngine {
    fn parse(&self, src: &str, fmt: PipelineFormat) -> Result<PipelineSpec, PipelineError> {
        PipelineParser::parse_str(src, fmt)
    }

    fn validate(&self, spec: &PipelineSpec) -> Result<Dag, PipelineError> {
        let dag = DagBuilder::build(spec)?;
        vars::validate_refs(spec, &dag)?;
        Ok(dag)
    }

    async fn run(&self, spec: PipelineSpec) -> Result<PipelineId, PipelineError> {
        self.executor.run(spec).await
    }

    async fn status(&self, id: PipelineId) -> Result<PipelineRun, PipelineError> {
        self.store.load_state(id)
    }

    async fn cancel(&self, id: PipelineId) -> Result<(), PipelineError> {
        self.executor.cancel(id).await
    }

    async fn resume(&self, id: PipelineId) -> Result<PipelineId, PipelineError> {
        // 1. Load run state.
        let mut run = self.store.load_state(id)?;

        // 2. Guard: already finished is a no-op.
        if run.state == PipelineState::Finished {
            return Ok(id);
        }

        // 3. Guard: rolled-back runs cannot be resumed.
        if run.state == PipelineState::RolledBack {
            return Err(PipelineError::ValidationError(
                "cannot resume a rolled-back run".to_string(),
            ));
        }

        // 4. Load the spec from disk.
        let spec = self.store.load_spec(id)?;

        // 5. Validate DAG + refs.
        let dag = DagBuilder::build(&spec)?;
        vars::validate_refs(&spec, &dag)?;

        // 6. Reset failed / running / cancelled steps to Pending so executor
        //    re-dispatches them.  Finished steps are preserved.
        for step_run in run.steps.values_mut() {
            if matches!(
                step_run.state,
                StepRunState::Failed | StepRunState::Running | StepRunState::Cancelled
            ) {
                step_run.state = StepRunState::Pending;
                step_run.attempts = 0;
                step_run.output = None;
                step_run.task_id = None;
                step_run.last_error = None;
            }
        }

        // 7. Transition to Running, audit Resumed.
        run.state = PipelineState::Running;
        run.finished_at = None;
        self.store.save_state(&run)?;
        self.store.append_audit(id, &AuditEvent::Resumed)?;

        // 8. Delegate to executor's resume_run.
        self.executor.resume_run(run, spec).await
    }

    async fn list(&self) -> Result<Vec<crate::id::PipelineId>, PipelineError> {
        self.store.list_runs()
    }
}
