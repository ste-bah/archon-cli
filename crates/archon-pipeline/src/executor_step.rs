use std::collections::HashMap;
use std::sync::Arc;

use archon_core::tasks::{SubmitRequest, TaskId, TaskResultStream, TaskSnapshot, TaskState};

use super::*;
use crate::spec::StepSpec;

struct StepExecutionContext {
    step: StepSpec,
    step_id: String,
    resolved_input: serde_json::Value,
    pipeline_id: PipelineId,
    max_attempts: u32,
}

enum StepAttemptOutcome {
    Retry,
    Done,
}

impl PipelineExecutor {
    /// Execute a single step with parallel-safe locking and retry support.
    ///
    /// The `run_lock` is acquired briefly for reads/writes but is NEVER held
    /// across any `TaskService` await point. On transient failures the step
    /// is retried up to `step.retry.max_attempts` times with backoff.
    pub(super) async fn execute_step_parallel(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        spec: &PipelineSpec,
        step_id: &str,
    ) -> Result<(), PipelineError> {
        let Some(ctx) = self
            .prepare_step_execution(run_lock.clone(), spec, step_id)
            .await?
        else {
            return Ok(());
        };

        for attempt in 1..=ctx.max_attempts {
            self.start_step_attempt(run_lock.clone(), &ctx, attempt)
                .await?;
            let task_id = self.submit_step_task(&ctx).await?;
            self.record_step_task_id(run_lock.clone(), &ctx, task_id)
                .await?;
            let snapshot = self.poll_step_task(&ctx, task_id).await?;
            match self
                .handle_step_snapshot(run_lock.clone(), &ctx, task_id, snapshot, attempt)
                .await?
            {
                StepAttemptOutcome::Retry => continue,
                StepAttemptOutcome::Done => return Ok(()),
            }
        }

        unreachable!("retry loop exited without returning")
    }

    async fn prepare_step_execution(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        spec: &PipelineSpec,
        step_id: &str,
    ) -> Result<Option<StepExecutionContext>, PipelineError> {
        let step = spec
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .cloned()
            .ok_or_else(|| PipelineError::MissingStep(step_id.to_string()))?;
        let max_attempts = step.retry.max_attempts.max(1);

        let (resolved_input, pipeline_id, outputs) = {
            let run = run_lock.lock().await;
            let outputs = finished_step_outputs(&run);
            let resolved = vars::substitute(&step.input, &outputs)?;
            (resolved, run.id, outputs)
        };

        if let Some(ref cond_expr) = step.condition
            && !condition::ConditionEvaluator::evaluate(cond_expr, &outputs)?
        {
            self.mark_step_skipped(run_lock, pipeline_id, step_id, None)
                .await?;
            return Ok(None);
        }

        Ok(Some(StepExecutionContext {
            step,
            step_id: step_id.to_string(),
            resolved_input,
            pipeline_id,
            max_attempts,
        }))
    }

    async fn start_step_attempt(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        attempt: u32,
    ) -> Result<(), PipelineError> {
        self.store.append_audit(
            ctx.pipeline_id,
            &AuditEvent::StepStarted {
                step: ctx.step_id.clone(),
                attempt,
            },
        )?;
        let mut run = run_lock.lock().await;
        if let Some(step_run) = run.steps.get_mut(&ctx.step_id) {
            step_run.state = StepRunState::Running;
            step_run.attempts = attempt;
        }
        self.store.save_state(&run)
    }

    async fn submit_step_task(&self, ctx: &StepExecutionContext) -> Result<TaskId, PipelineError> {
        self.task_service
            .submit(SubmitRequest {
                agent_name: ctx.step.agent.clone(),
                agent_version: None,
                input: ctx.resolved_input.clone(),
                owner: "pipeline".to_string(),
            })
            .await
            .map_err(|e| PipelineError::StepFailed {
                step: ctx.step_id.clone(),
                msg: e.to_string(),
            })
    }

    async fn record_step_task_id(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        task_id: TaskId,
    ) -> Result<(), PipelineError> {
        let mut run = run_lock.lock().await;
        if let Some(step_run) = run.steps.get_mut(&ctx.step_id) {
            step_run.task_id = Some(task_id);
        }
        self.store.save_state(&run)
    }

    async fn poll_step_task(
        &self,
        ctx: &StepExecutionContext,
        task_id: TaskId,
    ) -> Result<TaskSnapshot, PipelineError> {
        match tokio::time::timeout(
            ctx.step.timeout(),
            self.poll_task_until_terminal(ctx, task_id),
        )
        .await
        {
            Ok(inner) => inner,
            Err(_elapsed) => {
                let _ = self.task_service.cancel(task_id).await;
                Err(PipelineError::Timeout {
                    step: Some(ctx.step_id.clone()),
                })
            }
        }
    }

    async fn poll_task_until_terminal(
        &self,
        ctx: &StepExecutionContext,
        task_id: TaskId,
    ) -> Result<TaskSnapshot, PipelineError> {
        loop {
            let snap =
                self.task_service
                    .status(task_id)
                    .await
                    .map_err(|e| PipelineError::StepFailed {
                        step: ctx.step_id.clone(),
                        msg: e.to_string(),
                    })?;
            if snap.state.is_terminal() {
                return Ok(snap);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn handle_step_snapshot(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        task_id: TaskId,
        snapshot: TaskSnapshot,
        attempt: u32,
    ) -> Result<StepAttemptOutcome, PipelineError> {
        if snapshot.state == TaskState::Finished {
            self.finish_step(run_lock, ctx, task_id).await?;
            return Ok(StepAttemptOutcome::Done);
        }

        let error_msg = snapshot
            .error
            .unwrap_or_else(|| format!("step ended in state {:?}", snapshot.state));
        let err = PipelineError::StepFailed {
            step: ctx.step_id.clone(),
            msg: error_msg.clone(),
        };
        if err.is_transient() && attempt < ctx.max_attempts {
            self.schedule_retry(run_lock, ctx, &error_msg, attempt)
                .await?;
            return Ok(StepAttemptOutcome::Retry);
        }
        self.apply_terminal_step_failure(run_lock, ctx, error_msg, err)
            .await
    }

    async fn finish_step(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        task_id: TaskId,
    ) -> Result<(), PipelineError> {
        let result_stream = self
            .task_service
            .result(task_id, false)
            .await
            .map_err(|e| PipelineError::StepFailed {
                step: ctx.step_id.clone(),
                msg: e.to_string(),
            })?;
        let output = task_output_value(result_stream);
        let output_len = serde_json::to_string(&output).map(|s| s.len()).unwrap_or(0);
        self.store
            .write_checkpoint(ctx.pipeline_id, &ctx.step_id, &output)?;

        let mut run = run_lock.lock().await;
        if let Some(step_run) = run.steps.get_mut(&ctx.step_id) {
            step_run.state = StepRunState::Finished;
            step_run.output = Some(output);
        }
        self.store.save_state(&run)?;
        self.store.append_audit(
            ctx.pipeline_id,
            &AuditEvent::StepFinished {
                step: ctx.step_id.clone(),
                output_len,
            },
        )
    }

    async fn schedule_retry(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        error_msg: &str,
        attempt: u32,
    ) -> Result<(), PipelineError> {
        {
            let mut run = run_lock.lock().await;
            if let Some(step_run) = run.steps.get_mut(&ctx.step_id) {
                step_run.last_error = Some(error_msg.to_string());
            }
            self.store.save_state(&run)?;
        }
        self.store
            .delete_checkpoint(ctx.pipeline_id, &ctx.step_id)?;
        let delay = backoff::delay(
            ctx.step.retry.backoff,
            attempt,
            ctx.step.retry.base_delay_ms,
        );
        self.store.append_audit(
            ctx.pipeline_id,
            &AuditEvent::RetryScheduled {
                step: ctx.step_id.clone(),
                delay_ms: delay.as_millis() as u64,
            },
        )?;
        tokio::time::sleep(delay).await;
        Ok(())
    }

    async fn apply_terminal_step_failure(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        error_msg: String,
        err: PipelineError,
    ) -> Result<StepAttemptOutcome, PipelineError> {
        match rollback::handle_step_failure(ctx.step.on_failure) {
            FailureAction::Skip => {
                self.mark_step_skipped(run_lock, ctx.pipeline_id, &ctx.step_id, Some(error_msg))
                    .await?;
                Ok(StepAttemptOutcome::Done)
            }
            FailureAction::Fail | FailureAction::Rollback => {
                self.mark_step_failed(run_lock, ctx, error_msg).await?;
                Err(err)
            }
        }
    }

    async fn mark_step_skipped(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        pipeline_id: PipelineId,
        step_id: &str,
        last_error: Option<String>,
    ) -> Result<(), PipelineError> {
        let mut run = run_lock.lock().await;
        if let Some(step_run) = run.steps.get_mut(step_id) {
            step_run.state = StepRunState::Skipped;
            step_run.last_error = last_error;
        }
        self.store.save_state(&run)?;
        self.store.append_audit(
            pipeline_id,
            &AuditEvent::StepSkipped {
                step: step_id.to_string(),
            },
        )
    }

    async fn mark_step_failed(
        &self,
        run_lock: Arc<tokio::sync::Mutex<PipelineRun>>,
        ctx: &StepExecutionContext,
        error_msg: String,
    ) -> Result<(), PipelineError> {
        let mut run = run_lock.lock().await;
        if let Some(step_run) = run.steps.get_mut(&ctx.step_id) {
            step_run.state = StepRunState::Failed;
            step_run.last_error = Some(error_msg.clone());
        }
        self.store.save_state(&run)?;
        self.store.append_audit(
            ctx.pipeline_id,
            &AuditEvent::StepFailed {
                step: ctx.step_id.clone(),
                msg: error_msg,
            },
        )
    }
}

fn finished_step_outputs(run: &PipelineRun) -> HashMap<String, serde_json::Value> {
    run.steps
        .iter()
        .filter_map(|(sid, srun)| {
            (srun.state == StepRunState::Finished)
                .then(|| {
                    srun.output
                        .as_ref()
                        .map(|output| (sid.clone(), output.clone()))
                })
                .flatten()
        })
        .collect()
}

fn task_output_value(result_stream: TaskResultStream) -> serde_json::Value {
    match result_stream {
        TaskResultStream::Inline(s) => {
            serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::String(s))
        }
        TaskResultStream::File(path) => {
            serde_json::json!({"__archon_file_ref__": path.to_string_lossy()})
        }
    }
}
