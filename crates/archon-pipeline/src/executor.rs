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

#[path = "executor_step.rs"]
mod executor_step;

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
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
