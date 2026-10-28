use super::*;
use futures_util::FutureExt;

impl AgentSubagentExecutor {
    pub(super) async fn run_subagent_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.run_subagent_to_completion_with_system(subagent_id, request, Vec::new(), ctx, cancel)
            .await
    }

    pub(super) async fn run_subagent_to_completion_with_system(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.run_subagent_to_completion_with_system_metadata(
            subagent_id,
            request,
            system,
            ctx,
            cancel,
        )
        .await
        .map(|completion| completion.content)
    }

    pub(super) async fn run_subagent_in_slot(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
        row: &mut super::activity::ActivityRow,
    ) -> Result<archon_tools::subagent_executor::SubagentCompletion, ExecutorError> {
        let _capacity_permit = self
            .acquire_subagent_slot(&subagent_id, &request, &cancel, row)
            .await?;
        let ids = self.register_subagent_run(&subagent_id, &request).await?;
        // Held across everything below. The two statements that follow used to
        // be the only release, which covered every way a run can finish and no
        // way it can be abandoned — and an abandoned run left an id nothing,
        // including its own retry, could ever register again. See
        // `run_registration`.
        let mut registration = super::run_registration::RunRegistration::take(
            std::sync::Arc::clone(&self.subagent_manager),
            ids.manager_id.clone(),
        )
        .await;
        self.subagent_manager
            .lock()
            .await
            .set_parent(&ids.manager_id, ctx.subagent_id.as_deref());
        let result = self
            .run_registered_subagent_to_completion(&ids, request, system, ctx, cancel, row)
            .await;
        let progress = self
            .subagent_manager
            .lock()
            .await
            .get_progress(&ids.manager_id);
        let usage = progress.as_ref().map(|progress| {
            (
                progress
                    .input_tokens_available
                    .then_some(progress.cumulative_input_tokens),
                progress
                    .output_tokens_available
                    .then_some(progress.cumulative_output_tokens),
            )
        });
        if let Some((tokens_in, tokens_out)) = usage {
            self.usage_cache
                .lock()
                .await
                .insert(ids.cache_id.clone(), (tokens_in, tokens_out));
        }
        let result = result.map(|mut completion| {
            if let Some((tokens_in, tokens_out)) = usage {
                completion.tokens_in = tokens_in;
                completion.tokens_out = tokens_out;
            }
            completion
        });
        self.on_inner_complete(
            ids.cache_id,
            result
                .clone()
                .map(|completion| completion.content)
                .map_err(|err| err.to_string()),
        )
        .await;
        // Last, not before: a completion interrupted part-way through has still
        // left the entry `Running`, and the drop is what covers that.
        registration.settle();
        result
    }

    /// Take a subagent slot for `subagent_id`, waiting while every slot is in
    /// use (Issue 288). Taking the slot is reported (`admitted`): the dispatch
    /// clocks the host installed for this session start then, at once or
    /// after a wait. The wait is reported where it happens: it is logged and
    /// emitted as activity when it begins and when the slot is acquired. A
    /// call with no clock installed (an interactive call) is told so, never
    /// that clocks are stopped (Issue 322). The wait has no clock of its own:
    /// each slot holder is bounded by its own.
    async fn acquire_subagent_slot(
        &self,
        subagent_id: &str,
        request: &SubagentRequest,
        cancel: &CancellationToken,
        row: &mut super::activity::ActivityRow,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ExecutorError> {
        if let Ok(permit) = self.subagent_capacity.clone().try_acquire_owned() {
            // The dispatch clocks start when the slot is held, not before.
            archon_tools::subagent_dispatch_clock::admitted(subagent_id);
            return Ok(permit);
        }
        let paused = archon_tools::subagent_dispatch_clock::slot_wait(subagent_id);
        let clocks_paused = paused.as_ref().is_some_and(|pauses| !pauses.is_empty());
        let agent_type = request.subagent_type.as_deref().unwrap_or("subagent");
        let model = request.model.as_deref().unwrap_or(&self.parent_model);
        let limit = self.agent_config.max_subagent_concurrency.max(1);
        let clocks = if clocks_paused {
            "its dispatch clocks do not run until it starts"
        } else {
            "no dispatch clock is installed for it, so none is stopped"
        };
        let message =
            format!("{agent_type} waiting for a free subagent slot (all {limit} in use); {clocks}");
        tracing::info!(subagent_id, clocks_paused, "{message}");
        row.queued(agent_type, model, message);
        let waited = tokio::time::Instant::now();
        let permit = self.acquire_subagent_capacity(cancel).await?;
        archon_tools::subagent_dispatch_clock::admitted(subagent_id);
        drop(paused);
        let waited = waited.elapsed().as_secs();
        let message = if clocks_paused {
            format!(
                "{agent_type} acquired a subagent slot after waiting {waited}s; its dispatch clocks start now"
            )
        } else {
            format!("{agent_type} acquired a subagent slot after waiting {waited}s")
        };
        tracing::info!(subagent_id, "{message}");
        row.slot_acquired(agent_type, model, message);
        Ok(permit)
    }

    async fn acquire_subagent_capacity(
        &self,
        cancel: &CancellationToken,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ExecutorError> {
        tokio::select! {
            _ = cancel.cancelled() => {
                Err(ExecutorError::Internal("subagent cancelled".to_string()))
            }
            permit = self.subagent_capacity.clone().acquire_owned() => {
                permit.map_err(|_| ExecutorError::Internal(
                    "subagent capacity semaphore closed".to_string(),
                ))
            }
        }
    }

    async fn run_registered_subagent_to_completion(
        &self,
        ids: &super::run_prepare::RunIdentity,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
        row: &mut super::activity::ActivityRow,
    ) -> Result<archon_tools::subagent_executor::SubagentCompletion, ExecutorError> {
        self.fire_subagent_start_hooks(&ids.manager_id, &request, ctx.nested)
            .await;
        let (mut runner, context, _tool_cancellation) = if let Some(context) = &ids.resume_context {
            let built = self
                .restored_runner(ids, context, &cancel, ctx.cancel_parent.as_ref())
                .await?;
            (built.runner, Arc::clone(context), built.tool_cancellation)
        } else {
            let prepared = self
                .prepare_subagent_run(&ids.manager_id, &request, &ctx)
                .await?;
            let built = self
                .build_subagent_runner(ids, &request, &ctx, &prepared, &cancel)
                .await?;
            let mut runner = built.runner;
            let tool_cancellation = built.tool_cancellation;
            runner.set_request_system(system);
            let context = Arc::new(
                crate::subagent::runner::EffectiveRunContext::capture(
                    &mut runner,
                    prepared.tier,
                    built.worktree,
                    ctx.cancel_parent.clone(),
                )
                .await,
            );
            self.subagent_manager
                .lock()
                .await
                .remember_context(&ids.manager_id, ids.generation, Arc::clone(&context))
                .map_err(ExecutorError::Internal)?;
            (runner, context, tool_cancellation)
        };
        let activity_agent_type = context.activity_agent_type();
        let activity_model = runner.model().to_string();

        runner.set_activity_sink(row.scoped_sink());
        row.started(activity_agent_type, &activity_model);
        let runner_result = std::panic::AssertUnwindSafe(archon_tools::host_timeout::scope(
            context.host_timeout,
            async {
                if archon_tools::provider_recovery::enabled() {
                    archon_llm::provider_fault::defer_retry(
                        runner.run_with_completion(&request.prompt),
                    )
                    .await
                } else {
                    runner.run_with_completion(&request.prompt).await
                }
            },
        ))
        .catch_unwind()
        .await;
        let provider_source = match &runner_result {
            Ok(Err(error)) => error
                .chain()
                .find_map(|source| source.downcast_ref::<archon_llm::provider::LlmError>())
                .map(|error| {
                    archon_tools::subagent_executor::ProviderSource(Arc::new(error.clone()))
                }),
            _ => None,
        };
        let provider_failure = match &runner_result {
            Ok(Err(error)) => error
                .downcast_ref::<archon_llm::provider::LlmError>()
                .and_then(|error| match error {
                    archon_llm::provider::LlmError::RateLimited {
                        pipeline_retry_after_secs,
                        ..
                    } => Some((Some(429), *pipeline_retry_after_secs, true)),
                    archon_llm::provider::LlmError::Overloaded {
                        status,
                        retry_after_secs,
                        ..
                    } => Some((*status, *retry_after_secs, true)),
                    archon_llm::provider::LlmError::Server { status, .. }
                        if matches!(status, 429 | 529) =>
                    {
                        Some((Some(*status), None, true))
                    }
                    _ => None,
                }),
            _ => None,
        };
        let provider_failure =
            provider_failure.or_else(|| provider_source.as_ref().map(|_| (None, None, false)));
        let inner_result = match runner_result {
            Ok(result) => result
                .map(
                    |completion| archon_tools::subagent_executor::SubagentCompletion {
                        content: completion.content,
                        stop_reason: completion.stop_reason,
                        tokens_in: completion.tokens_in,
                        tokens_out: completion.tokens_out,
                    },
                )
                .map_err(|e| format!("Subagent failed: {e:#}")),
            Err(payload) => Err(format!(
                "Subagent panicked: {}",
                super::activity::panic_message(payload.as_ref())
            )),
        };
        row.finished(
            activity_agent_type,
            &activity_model,
            &inner_result
                .as_ref()
                .map(|c| c.content.clone())
                .map_err(Clone::clone),
        );

        match (inner_result, provider_failure) {
            (Err(message), Some((status, retry_after_secs, overloaded))) => {
                Err(ExecutorError::ProviderFailure {
                    message,
                    status,
                    retry_after_secs,
                    overloaded,
                    source: provider_source,
                })
            }
            (Err(message), None) => Err(ExecutorError::Internal(message)),
            (Ok(completion), _) => Ok(completion),
        }
    }
}

#[cfg(test)]
#[path = "run_activity_tests.rs"]
mod activity_tests;

#[cfg(test)]
#[path = "run_capacity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "run_followup_tests.rs"]
mod followup_tests;
