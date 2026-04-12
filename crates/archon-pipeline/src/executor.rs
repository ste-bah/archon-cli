//! Parallel pipeline executor.
//!
//! Walks the DAG level-by-level, dispatching steps within each level
//! concurrently (up to `max_parallelism`) via [`futures_util::stream::buffer_unordered`].
//! Step outputs are collected and fed into downstream variable substitution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use futures_util::stream::{self, StreamExt};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::backoff;
use crate::condition;
use crate::dag::DagBuilder;
use crate::error::PipelineError;
use crate::id::PipelineId;
use crate::rollback::{self, FailureAction, RollbackEngine};
use crate::run::{PipelineRun, PipelineState, StepRun, StepRunState};
use crate::spec::PipelineSpec;
use crate::store::{AuditEvent, PipelineStateStore};
use crate::vars;

/// Parallel pipeline executor.
///
/// Builds the DAG, validates variable references, then walks levels
/// sequentially.  Within each level, steps are dispatched concurrently
/// (up to [`PipelineSpec::max_parallelism`]) through the injected
/// [`archon_core::tasks::TaskService`].
#[derive(Clone)]
pub struct PipelineExecutor {
    store: Arc<PipelineStateStore>,
    task_service: Arc<dyn archon_core::tasks::TaskService>,
    rollback_engine: Arc<RollbackEngine>,
    runs_in_flight: Arc<DashMap<PipelineId, CancellationToken>>,
}

impl PipelineExecutor {
    /// Create a new executor backed by the given state store and task service.
    pub fn new(
        store: Arc<PipelineStateStore>,
        task_service: Arc<dyn archon_core::tasks::TaskService>,
    ) -> Self {
        let rollback_engine = Arc::new(RollbackEngine::new(store.clone()));
        Self {
            store,
            task_service,
            rollback_engine,
            runs_in_flight: Arc::new(DashMap::new()),
        }
    }

    /// Execute a pipeline specification to completion.
    ///
    /// Returns the [`PipelineId`] of the completed run on success, or a
    /// [`PipelineError`] if any step fails.
    pub async fn run(&self, spec: PipelineSpec) -> Result<PipelineId, PipelineError> {
        // 1. Build and validate the DAG.
        let dag = DagBuilder::build(&spec)?;
        vars::validate_refs(&spec, &dag)?;

        // 2. Compute spec hash.
        let spec_hash = hex::encode(Sha256::digest(
            serde_json::to_vec(&spec).expect("spec serialization cannot fail"),
        ));

        // 3. Create the pipeline run.
        let id = PipelineId::new();
        let mut steps = HashMap::new();
        for step in &spec.steps {
            steps.insert(
                step.id.clone(),
                StepRun {
                    task_id: None,
                    state: StepRunState::Pending,
                    output: None,
                    attempts: 0,
                    last_error: None,
                },
            );
        }

        let run = PipelineRun {
            id,
            spec_hash: spec_hash.clone(),
            state: PipelineState::Pending,
            steps,
            started_at: Utc::now(),
            finished_at: None,
        };

        // 4. Persist initial state + spec for resume-from-checkpoint.
        self.store.create(&run)?;
        self.store.save_spec(id, &spec)?;
        self.store
            .append_audit(id, &AuditEvent::Started { spec_hash })?;

        // 5. Wrap run in Arc<Mutex> for shared access across parallel tasks.
        let run_lock = Arc::new(tokio::sync::Mutex::new(run));

        // 6. Transition to Running.
        {
            let mut run = run_lock.lock().await;
            run.state = PipelineState::Running;
            self.store.save_state(&run)?;
        }

        // 7. Create cancellation token for this run.
        let cancel_token = CancellationToken::new();
        self.runs_in_flight.insert(id, cancel_token.clone());

        // 8. Walk the DAG level-by-level, wrapped in global timeout.
        let level_walker = async {
            for level in &dag.levels {
                // Check cancellation at the start of each level.
                if cancel_token.is_cancelled() {
                    let mut run = run_lock.lock().await;
                    run.state = PipelineState::Cancelled;
                    run.finished_at = Some(Utc::now());
                    let _ = self.store.save_state(&run);
                    let _ = self.store.append_audit(id, &AuditEvent::Cancelled);
                    return Err(PipelineError::Timeout { step: None });
                }

                let concurrency = spec.max_parallelism.max(1) as usize;

                let results: Vec<Result<(), PipelineError>> = stream::iter(level.iter().cloned())
                    .map(|step_id| {
                        let executor = self.clone();
                        let spec = spec.clone();
                        let run_lock = run_lock.clone();
                        async move {
                            executor
                                .execute_step_parallel(run_lock, &spec, &step_id)
                                .await
                        }
                    })
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;

                // Check for any errors in this level.
                let errors: Vec<PipelineError> =
                    results.into_iter().filter_map(|r| r.err()).collect();
                if !errors.is_empty() {
                    let first_error = errors.into_iter().next().unwrap();

                    // Determine if any failing step in this level wants rollback.
                    let needs_rollback = {
                        let run = run_lock.lock().await;
                        level.iter().any(|step_id| {
                            let is_failed = run
                                .steps
                                .get(step_id)
                                .map(|sr| sr.state == StepRunState::Failed)
                                .unwrap_or(false);
                            if !is_failed {
                                return false;
                            }
                            spec.steps
                                .iter()
                                .find(|s| s.id == *step_id)
                                .map(|s| {
                                    matches!(
                                        rollback::handle_step_failure(s.on_failure),
                                        FailureAction::Rollback
                                    )
                                })
                                .unwrap_or(false)
                        })
                    };

                    if needs_rollback {
                        let mut run = run_lock.lock().await;
                        self.rollback_engine.rollback(&mut run, &dag)?;
                        return Err(first_error);
                    } else {
                        // Fail policy: preserve state.
                        let mut run = run_lock.lock().await;
                        run.state = PipelineState::Failed;
                        run.finished_at = Some(Utc::now());
                        self.store.save_state(&run)?;
                        self.store.append_audit(id, &AuditEvent::Finished)?;
                        return Err(first_error);
                    }
                }

                // Save state after each level completes.
                {
                    let run = run_lock.lock().await;
                    self.store.save_state(&run)?;
                }
            }

            // All steps succeeded.
            {
                let mut run = run_lock.lock().await;
                run.state = PipelineState::Finished;
                run.finished_at = Some(Utc::now());
                self.store.save_state(&run)?;
                self.store.append_audit(id, &AuditEvent::Finished)?;
            }

            Ok(id)
        };

        let result = tokio::time::timeout(spec.global_timeout(), level_walker).await;

        // Clean up the in-flight entry regardless of outcome.
        self.runs_in_flight.remove(&id);

        match result {
            Ok(inner) => inner,
            Err(_elapsed) => {
                // Global timeout fired — mark as Cancelled and clean up
                // checkpoints, but preserve the state file for inspection.
                let mut run = run_lock.lock().await;
                run.state = PipelineState::Cancelled;
                run.finished_at = Some(Utc::now());
                let _ = self.store.save_state(&run);
                let _ = self.store.append_audit(id, &AuditEvent::Cancelled);
                // Attempt rollback (deletes checkpoints + state file and
                // sets state to RolledBack internally), then re-persist the
                // Cancelled state so load_state still works.
                let _ = self.rollback_engine.rollback(&mut run, &dag);
                run.state = PipelineState::Cancelled;
                let _ = self.store.save_state(&run);
                Err(PipelineError::Timeout { step: None })
            }
        }
    }

    /// Cancel a running pipeline by its ID.
    ///
    /// Looks up the [`CancellationToken`] for the given run and triggers it.
    /// The main run loop checks the token at each level boundary and exits.
    /// Returns `Ok(())` if the token was found and cancelled, or
    /// [`PipelineError::MissingStep`] if the run is not in flight.
    pub async fn cancel(&self, id: PipelineId) -> Result<(), PipelineError> {
        let entry = self
            .runs_in_flight
            .get(&id)
            .ok_or_else(|| PipelineError::ValidationError(format!("no in-flight run {id}")))?;
        entry.value().cancel();
        Ok(())
    }

    /// Resume a pipeline run from its current checkpoint state.
    ///
    /// Steps already `Finished` are skipped (their outputs are already in
    /// `run.steps`).  Steps that were `Failed` should have been reset to
    /// `Pending` by the caller before invoking this method.
    ///
    /// This method reuses the same DAG-walking and step-dispatch machinery
    /// as [`run`], but starts from an existing `PipelineRun` rather than
    /// creating a new one.
    pub async fn resume_run(
        &self,
        run: PipelineRun,
        spec: PipelineSpec,
    ) -> Result<PipelineId, PipelineError> {
        let id = run.id;

        // Build DAG.
        let dag = DagBuilder::build(&spec)?;

        // Wrap in Arc<Mutex>.
        let run_lock = Arc::new(tokio::sync::Mutex::new(run));

        // Create cancellation token.
        let cancel_token = CancellationToken::new();
        self.runs_in_flight.insert(id, cancel_token.clone());

        // Walk the DAG level-by-level.
        let level_walker = async {
            for level in &dag.levels {
                // Check cancellation.
                if cancel_token.is_cancelled() {
                    let mut run = run_lock.lock().await;
                    run.state = PipelineState::Cancelled;
                    run.finished_at = Some(Utc::now());
                    let _ = self.store.save_state(&run);
                    let _ = self.store.append_audit(id, &AuditEvent::Cancelled);
                    return Err(PipelineError::Timeout { step: None });
                }

                // Filter: only dispatch steps that are NOT Finished.
                let pending_steps: Vec<String> = {
                    let run = run_lock.lock().await;
                    level
                        .iter()
                        .filter(|step_id| {
                            run.steps
                                .get(*step_id)
                                .map(|sr| sr.state != StepRunState::Finished)
                                .unwrap_or(true)
                        })
                        .cloned()
                        .collect()
                };

                if pending_steps.is_empty() {
                    continue; // Entire level already done.
                }

                let concurrency = spec.max_parallelism.max(1) as usize;

                let results: Vec<Result<(), PipelineError>> =
                    stream::iter(pending_steps.iter().cloned())
                        .map(|step_id| {
                            let executor = self.clone();
                            let spec = spec.clone();
                            let run_lock = run_lock.clone();
                            async move {
                                executor
                                    .execute_step_parallel(run_lock, &spec, &step_id)
                                    .await
                            }
                        })
                        .buffer_unordered(concurrency)
                        .collect()
                        .await;

                // Check errors.
                let errors: Vec<PipelineError> =
                    results.into_iter().filter_map(|r| r.err()).collect();
                if !errors.is_empty() {
                    let first_error = errors.into_iter().next().unwrap();

                    let needs_rollback = {
                        let run = run_lock.lock().await;
                        level.iter().any(|step_id| {
                            let is_failed = run
                                .steps
                                .get(step_id)
                                .map(|sr| sr.state == StepRunState::Failed)
                                .unwrap_or(false);
                            if !is_failed {
                                return false;
                            }
                            spec.steps
                                .iter()
                                .find(|s| s.id == *step_id)
                                .map(|s| {
                                    matches!(
                                        rollback::handle_step_failure(s.on_failure),
                                        FailureAction::Rollback
                                    )
                                })
                                .unwrap_or(false)
                        })
                    };

                    if needs_rollback {
                        let mut run = run_lock.lock().await;
                        self.rollback_engine.rollback(&mut run, &dag)?;
                        return Err(first_error);
                    } else {
                        let mut run = run_lock.lock().await;
                        run.state = PipelineState::Failed;
                        run.finished_at = Some(Utc::now());
                        self.store.save_state(&run)?;
                        self.store.append_audit(id, &AuditEvent::Finished)?;
                        return Err(first_error);
                    }
                }

                // Save state after each level.
                {
                    let run = run_lock.lock().await;
                    self.store.save_state(&run)?;
                }
            }

            // All steps succeeded.
            {
                let mut run = run_lock.lock().await;
                run.state = PipelineState::Finished;
                run.finished_at = Some(Utc::now());
                self.store.save_state(&run)?;
                self.store.append_audit(id, &AuditEvent::Finished)?;
            }

            Ok(id)
        };

        let result = tokio::time::timeout(spec.global_timeout(), level_walker).await;

        self.runs_in_flight.remove(&id);

        match result {
            Ok(inner) => inner,
            Err(_elapsed) => {
                let mut run = run_lock.lock().await;
                run.state = PipelineState::Cancelled;
                run.finished_at = Some(Utc::now());
                let _ = self.store.save_state(&run);
                let _ = self.store.append_audit(id, &AuditEvent::Cancelled);
                let _ = self.rollback_engine.rollback(&mut run, &dag);
                run.state = PipelineState::Cancelled;
                let _ = self.store.save_state(&run);
                Err(PipelineError::Timeout { step: None })
            }
        }
    }

    /// Execute a single step with parallel-safe locking and retry support.
    ///
    /// The `run_lock` is acquired briefly for reads/writes but is NEVER held
    /// across any `TaskService` await point.  On transient failures the step
    /// is retried up to `step.retry.max_attempts` times with the configured
    /// backoff strategy.
    async fn execute_step_parallel(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        spec: &PipelineSpec,
        step_id: &str,
    ) -> Result<(), PipelineError> {
        // 1. Find the step spec (no lock needed — spec is immutable).
        let step = spec
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .ok_or_else(|| PipelineError::MissingStep(step_id.to_string()))?;

        let max_attempts = step.retry.max_attempts.max(1);

        // 2. Lock briefly: collect completed outputs for variable substitution
        //    and extract the pipeline id.  This only needs to happen once since
        //    upstream outputs don't change across retries of the same step.
        let (resolved_input, pipeline_id, outputs_snapshot) = {
            let run = run_lock.lock().await;
            let mut outputs: HashMap<String, serde_json::Value> = HashMap::new();
            for (sid, srun) in &run.steps {
                if srun.state == StepRunState::Finished {
                    if let Some(ref output) = srun.output {
                        outputs.insert(sid.clone(), output.clone());
                    }
                }
            }
            let resolved = vars::substitute(&step.input, &outputs)?;
            (resolved, run.id, outputs)
        };
        // Lock released.

        // 2b. Check condition BEFORE entering retry loop.
        if let Some(ref cond_expr) = step.condition {
            let should_run =
                condition::ConditionEvaluator::evaluate(cond_expr, &outputs_snapshot)?;
            if !should_run {
                // Mark step as Skipped.
                {
                    let mut run = run_lock.lock().await;
                    if let Some(step_run) = run.steps.get_mut(step_id) {
                        step_run.state = StepRunState::Skipped;
                    }
                    self.store.save_state(&run)?;
                }
                self.store.append_audit(
                    pipeline_id,
                    &AuditEvent::StepSkipped {
                        step: step_id.to_string(),
                    },
                )?;
                return Ok(());
            }
        }

        // 3. Retry loop.
        for attempt in 1..=max_attempts {
            // 3a. Audit step start with current attempt number.
            self.store.append_audit(
                pipeline_id,
                &AuditEvent::StepStarted {
                    step: step_id.to_string(),
                    attempt,
                },
            )?;

            // 3b. Lock briefly: mark step as Running, record attempt count.
            {
                let mut run = run_lock.lock().await;
                if let Some(step_run) = run.steps.get_mut(step_id) {
                    step_run.state = StepRunState::Running;
                    step_run.attempts = attempt;
                }
                self.store.save_state(&run)?;
            }
            // Lock released.

            // 3c. Submit to TaskService (NO lock held).
            let task_id = self
                .task_service
                .submit(archon_core::tasks::SubmitRequest {
                    agent_name: step.agent.clone(),
                    agent_version: None,
                    input: resolved_input.clone(),
                    owner: "pipeline".to_string(),
                })
                .await
                .map_err(|e| PipelineError::StepFailed {
                    step: step_id.to_string(),
                    msg: e.to_string(),
                })?;

            // 3d. Lock briefly: store task_id on the StepRun.
            {
                let mut run = run_lock.lock().await;
                if let Some(step_run) = run.steps.get_mut(step_id) {
                    step_run.task_id = Some(task_id);
                }
                self.store.save_state(&run)?;
            }
            // Lock released.

            // 3e. Poll until terminal (NO lock held), wrapped in per-step timeout.
            let poll_result = tokio::time::timeout(step.timeout(), async {
                loop {
                    let snap = self
                        .task_service
                        .status(task_id)
                        .await
                        .map_err(|e| PipelineError::StepFailed {
                            step: step_id.to_string(),
                            msg: e.to_string(),
                        })?;
                    if snap.state.is_terminal() {
                        break Ok::<_, PipelineError>(snap);
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await;

            let snapshot = match poll_result {
                Ok(inner) => inner?,
                Err(_elapsed) => {
                    // Per-step timeout: cancel the task and return Timeout error.
                    let _ = self.task_service.cancel(task_id).await;
                    return Err(PipelineError::Timeout {
                        step: Some(step_id.to_string()),
                    });
                }
            };

            // 3f. Handle terminal state.
            match snapshot.state {
                archon_core::tasks::TaskState::Finished => {
                    // Get result from task service (NO lock held).
                    let result_stream = self
                        .task_service
                        .result(task_id, false)
                        .await
                        .map_err(|e| PipelineError::StepFailed {
                            step: step_id.to_string(),
                            msg: e.to_string(),
                        })?;

                    let output = match result_stream {
                        archon_core::tasks::TaskResultStream::Inline(s) => {
                            serde_json::from_str::<serde_json::Value>(&s)
                                .unwrap_or_else(|_| serde_json::Value::String(s))
                        }
                        archon_core::tasks::TaskResultStream::File(path) => {
                            serde_json::json!({"__archon_file_ref__": path.to_string_lossy()})
                        }
                    };

                    // Write checkpoint (thread-safe atomic file op).
                    let output_len = serde_json::to_string(&output)
                        .map(|s| s.len())
                        .unwrap_or(0);
                    self.store
                        .write_checkpoint(pipeline_id, step_id, &output)?;

                    // Lock briefly: update StepRun state + save.
                    {
                        let mut run = run_lock.lock().await;
                        if let Some(step_run) = run.steps.get_mut(step_id) {
                            step_run.state = StepRunState::Finished;
                            step_run.output = Some(output);
                        }
                        self.store.save_state(&run)?;
                    }
                    // Lock released.

                    // Audit step finished.
                    self.store.append_audit(
                        pipeline_id,
                        &AuditEvent::StepFinished {
                            step: step_id.to_string(),
                            output_len,
                        },
                    )?;

                    return Ok(());
                }
                // Failed / Cancelled / Corrupted
                _ => {
                    let error_msg = snapshot
                        .error
                        .unwrap_or_else(|| format!("step ended in state {:?}", snapshot.state));

                    let err = PipelineError::StepFailed {
                        step: step_id.to_string(),
                        msg: error_msg.clone(),
                    };

                    // Decide whether to retry.
                    if err.is_transient() && attempt < max_attempts {
                        // Lock briefly: store last_error, save state.
                        {
                            let mut run = run_lock.lock().await;
                            if let Some(step_run) = run.steps.get_mut(step_id) {
                                step_run.last_error = Some(error_msg.clone());
                            }
                            self.store.save_state(&run)?;
                        }
                        // Lock released.

                        // Clean up partial checkpoint from failed attempt.
                        self.store.delete_checkpoint(pipeline_id, step_id)?;

                        // Compute backoff delay.
                        let delay = backoff::delay(
                            step.retry.backoff,
                            attempt,
                            step.retry.base_delay_ms,
                        );

                        // Audit retry scheduled.
                        self.store.append_audit(
                            pipeline_id,
                            &AuditEvent::RetryScheduled {
                                step: step_id.to_string(),
                                delay_ms: delay.as_millis() as u64,
                            },
                        )?;

                        // Sleep for backoff duration.
                        tokio::time::sleep(delay).await;

                        // Continue to next attempt.
                        continue;
                    }

                    // No more retries — dispatch based on on_failure policy.
                    let action = rollback::handle_step_failure(step.on_failure);

                    match action {
                        FailureAction::Skip => {
                            // Mark step as Skipped instead of Failed.
                            {
                                let mut run = run_lock.lock().await;
                                if let Some(step_run) = run.steps.get_mut(step_id) {
                                    step_run.state = StepRunState::Skipped;
                                    step_run.last_error = Some(error_msg.clone());
                                }
                                self.store.save_state(&run)?;
                            }
                            // Lock released.

                            // Audit step skipped.
                            self.store.append_audit(
                                pipeline_id,
                                &AuditEvent::StepSkipped {
                                    step: step_id.to_string(),
                                },
                            )?;

                            return Ok(()); // Skip = continue pipeline
                        }
                        FailureAction::Fail | FailureAction::Rollback => {
                            // Mark step as Failed.
                            {
                                let mut run = run_lock.lock().await;
                                if let Some(step_run) = run.steps.get_mut(step_id) {
                                    step_run.state = StepRunState::Failed;
                                    step_run.last_error = Some(error_msg.clone());
                                }
                                self.store.save_state(&run)?;
                            }
                            // Lock released.

                            // Audit step failed.
                            self.store.append_audit(
                                pipeline_id,
                                &AuditEvent::StepFailed {
                                    step: step_id.to_string(),
                                    msg: error_msg.clone(),
                                },
                            )?;

                            return Err(err);
                        }
                    }
                }
            }
        }

        // Should be unreachable — the loop always returns from the match arms.
        unreachable!("retry loop exited without returning")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;
    use tempfile::TempDir;
    use futures_util::Stream;

    use archon_core::tasks::{
        SubmitRequest, TaskError, TaskEvent, TaskFilter, TaskId, TaskResultStream, TaskService,
        TaskSnapshot, TaskState,
    };

    use crate::spec::{BackoffKind, OnFailurePolicy, PipelineSpec, RetrySpec, StepSpec};

    // -----------------------------------------------------------------------
    // Mock TaskService
    // -----------------------------------------------------------------------

    /// Recorded call from submit().
    #[derive(Debug, Clone)]
    struct SubmitRecord {
        agent_name: String,
        input: serde_json::Value,
        task_id: TaskId,
    }

    /// Configurable mock task service for pipeline executor tests.
    struct MockTaskService {
        /// Maps agent_name to (output_value, should_fail).
        responses: Mutex<HashMap<String, (serde_json::Value, bool)>>,
        /// Tracks all submit calls in order.
        submissions: Mutex<Vec<SubmitRecord>>,
        call_count: AtomicU32,
    }

    impl MockTaskService {
        fn new(responses: HashMap<String, (serde_json::Value, bool)>) -> Self {
            Self {
                responses: Mutex::new(responses),
                submissions: Mutex::new(Vec::new()),
                call_count: AtomicU32::new(0),
            }
        }

        /// Return all recorded submissions.
        fn get_submissions(&self) -> Vec<SubmitRecord> {
            self.submissions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TaskService for MockTaskService {
        async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let task_id = TaskId::new();
            self.submissions.lock().unwrap().push(SubmitRecord {
                agent_name: req.agent_name,
                input: req.input,
                task_id,
            });
            Ok(task_id)
        }

        async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
            // Find the submission by task_id to get the agent_name.
            let submissions = self.submissions.lock().unwrap();
            let record = submissions
                .iter()
                .find(|r| r.task_id == id)
                .ok_or(TaskError::NotFound(id))?;

            let responses = self.responses.lock().unwrap();
            let (_output, should_fail) = responses
                .get(&record.agent_name)
                .ok_or(TaskError::NotFound(id))?;

            let state = if *should_fail {
                TaskState::Failed
            } else {
                TaskState::Finished
            };

            Ok(TaskSnapshot {
                id,
                agent_name: record.agent_name.clone(),
                state,
                progress_pct: None,
                created_at: Utc::now(),
                started_at: Some(Utc::now()),
                finished_at: Some(Utc::now()),
                error: if *should_fail {
                    Some("mock failure".to_string())
                } else {
                    None
                },
            })
        }

        async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
            let submissions = self.submissions.lock().unwrap();
            let record = submissions
                .iter()
                .find(|r| r.task_id == id)
                .ok_or(TaskError::NotFound(id))?;

            let responses = self.responses.lock().unwrap();
            let (output, should_fail) = responses
                .get(&record.agent_name)
                .ok_or(TaskError::NotFound(id))?;

            if *should_fail {
                return Err(TaskError::NotFound(id));
            }

            Ok(TaskResultStream::Inline(
                serde_json::to_string(output).unwrap(),
            ))
        }

        async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
            Err(TaskError::Unimplemented)
        }

        async fn subscribe_events(
            &self,
            _id: TaskId,
            _from_seq: u64,
        ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError> {
            Err(TaskError::Unimplemented)
        }

        async fn list(&self, _filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
            Err(TaskError::Unimplemented)
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_spec(steps: Vec<(&str, &str, Vec<&str>, serde_json::Value)>) -> PipelineSpec {
        PipelineSpec {
            name: "test-pipeline".to_string(),
            version: "1.0".to_string(),
            global_timeout_secs: 3600,
            max_parallelism: 1,
            steps: steps
                .into_iter()
                .map(|(id, agent, deps, input)| StepSpec {
                    id: id.to_string(),
                    agent: agent.to_string(),
                    input,
                    depends_on: deps.into_iter().map(|d| d.to_string()).collect(),
                    retry: RetrySpec {
                        max_attempts: 1,
                        backoff: BackoffKind::Exponential,
                        base_delay_ms: 1000,
                    },
                    timeout_secs: 1800,
                    condition: None,
                    on_failure: OnFailurePolicy::Fail,
                })
                .collect(),
        }
    }

    /// Read the audit log lines from disk for a given pipeline run.
    fn read_audit_lines(store_root: &std::path::Path, id: PipelineId) -> Vec<serde_json::Value> {
        let audit_path = store_root.join(id.to_string()).join("audit.log");
        let raw = std::fs::read_to_string(&audit_path).unwrap_or_default();
        raw.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).expect("valid audit JSON"))
            .collect()
    }

    /// Check whether a checkpoint file exists for a given step.
    fn checkpoint_exists(store_root: &std::path::Path, id: PipelineId, step_id: &str) -> bool {
        store_root
            .join(id.to_string())
            .join("checkpoints")
            .join(format!("{step_id}.json"))
            .exists()
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn three_step_linear_success() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(PipelineStateStore::new(tmp.path()));

        // A -> B -> C, each outputs {"count": N}
        let mut responses = HashMap::new();
        responses.insert("agent-a".to_string(), (json!({"count": 1}), false));
        responses.insert("agent-b".to_string(), (json!({"count": 2}), false));
        responses.insert("agent-c".to_string(), (json!({"count": 3}), false));

        let mock = Arc::new(MockTaskService::new(responses));
        let executor = PipelineExecutor::new(store.clone(), mock.clone());

        let spec = make_spec(vec![
            ("A", "agent-a", vec![], json!({})),
            ("B", "agent-b", vec!["A"], json!({"prev": "${A.output}"})),
            ("C", "agent-c", vec!["B"], json!({"prev": "${B.output}"})),
        ]);

        let id = executor.run(spec).await.expect("pipeline should succeed");

        // Verify pipeline state.
        let run = store.load_state(id).expect("state should load");
        assert_eq!(run.state, PipelineState::Finished);
        assert!(run.finished_at.is_some());

        // All 3 steps should be Finished.
        for step_id in ["A", "B", "C"] {
            let step_run = run.steps.get(step_id).expect("step should exist");
            assert_eq!(step_run.state, StepRunState::Finished, "step {step_id}");
            assert!(step_run.output.is_some(), "step {step_id} should have output");
        }

        // 3 checkpoints should exist.
        for step_id in ["A", "B", "C"] {
            assert!(
                checkpoint_exists(tmp.path(), id, step_id),
                "checkpoint for {step_id} should exist"
            );
        }

        // Audit log: Started + 3x(StepStarted + StepFinished) + Finished = 8 lines.
        let audit = read_audit_lines(tmp.path(), id);
        assert_eq!(audit.len(), 8, "expected 8 audit lines, got {}", audit.len());

        // First event is Started, last is Finished.
        assert_eq!(audit[0]["type"], "started");
        assert_eq!(audit[audit.len() - 1]["type"], "finished");

        // 3 submit calls.
        assert_eq!(mock.call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn failure_mid_pipeline() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(PipelineStateStore::new(tmp.path()));

        // A succeeds, B fails, C should never run.
        let mut responses = HashMap::new();
        responses.insert("agent-a".to_string(), (json!({"ok": true}), false));
        responses.insert("agent-b".to_string(), (json!(null), true)); // fails
        responses.insert("agent-c".to_string(), (json!({"ok": true}), false));

        let mock = Arc::new(MockTaskService::new(responses));
        let executor = PipelineExecutor::new(store.clone(), mock.clone());

        let spec = make_spec(vec![
            ("A", "agent-a", vec![], json!({})),
            ("B", "agent-b", vec!["A"], json!({})),
            ("C", "agent-c", vec!["B"], json!({})),
        ]);

        let err = executor.run(spec).await.expect_err("pipeline should fail");
        match err {
            PipelineError::StepFailed { step, .. } => {
                assert_eq!(step, "B", "step B should be the failure");
            }
            other => panic!("expected StepFailed, got: {other:?}"),
        }

        // Load the persisted state — the executor saves before returning the error.
        let runs = store.list_runs().expect("list runs");
        assert_eq!(runs.len(), 1);
        let id = runs[0];

        let run = store.load_state(id).expect("state should load");
        assert_eq!(run.state, PipelineState::Failed);

        // A should be Finished, B should be Failed, C should be Pending.
        assert_eq!(run.steps["A"].state, StepRunState::Finished);
        assert_eq!(run.steps["B"].state, StepRunState::Failed);
        assert_eq!(run.steps["C"].state, StepRunState::Pending);

        // Checkpoint for A exists, not for B or C.
        assert!(checkpoint_exists(tmp.path(), id, "A"));
        assert!(!checkpoint_exists(tmp.path(), id, "B"));
        assert!(!checkpoint_exists(tmp.path(), id, "C"));

        // Audit should contain StepFailed for B.
        let audit = read_audit_lines(tmp.path(), id);
        let has_step_failed = audit
            .iter()
            .any(|e| e["type"] == "step_failed" && e["step"] == "B");
        assert!(has_step_failed, "audit should contain step_failed for B");

        // Only 2 submit calls (A and B), C was never submitted.
        assert_eq!(mock.call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn variable_substitution_carries_through() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(PipelineStateStore::new(tmp.path()));

        // A outputs {"data": 42}, B's input references ${A.output.data}.
        let mut responses = HashMap::new();
        responses.insert("agent-a".to_string(), (json!({"data": 42}), false));
        responses.insert("agent-b".to_string(), (json!({"result": "ok"}), false));

        let mock = Arc::new(MockTaskService::new(responses));
        let executor = PipelineExecutor::new(store.clone(), mock.clone());

        let spec = make_spec(vec![
            ("A", "agent-a", vec![], json!({})),
            (
                "B",
                "agent-b",
                vec!["A"],
                json!({"ref": "${A.output.data}"}),
            ),
        ]);

        let _id = executor.run(spec).await.expect("pipeline should succeed");

        // Verify the mock received the resolved input for B.
        let submissions = mock.get_submissions();
        assert_eq!(submissions.len(), 2);

        let b_submission = submissions
            .iter()
            .find(|s| s.agent_name == "agent-b")
            .expect("agent-b should have been submitted");

        // The key check: ${A.output.data} should resolve to the number 42,
        // preserving type (not the string "42").
        assert_eq!(
            b_submission.input,
            json!({"ref": 42}),
            "variable substitution should resolve ${{A.output.data}} to the number 42"
        );
    }
}
