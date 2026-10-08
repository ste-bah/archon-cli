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
        // Every way the call ends after its activity row opens closes the row
        // (Issue 322): the run's completion, `settle` for an error before it,
        // and the row's drop for an abandoned call.
        let mut row = self.activity_row(&subagent_id);
        let result = self
            .run_subagent_in_slot(subagent_id, request, system, ctx, cancel.clone(), &mut row)
            .await;
        row.settle(&result, &cancel);
        result
    }

    async fn run_subagent_in_slot(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
        row: &mut super::activity::ActivityRow,
    ) -> Result<String, ExecutorError> {
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
        self.on_inner_complete(ids.cache_id, result.clone().map_err(|err| err.to_string()))
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
    ) -> Result<String, ExecutorError> {
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
            runner.run(&request.prompt),
        ))
        .catch_unwind()
        .await;
        let inner_result = match runner_result {
            Ok(result) => result.map_err(|e| format!("Subagent failed: {e}")),
            Err(payload) => Err(format!(
                "Subagent panicked: {}",
                super::activity::panic_message(payload.as_ref())
            )),
        };
        row.finished(activity_agent_type, &activity_model, &inner_result);

        inner_result.map_err(ExecutorError::Internal)
    }
}

#[cfg(test)]
#[path = "run_activity_tests.rs"]
mod activity_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::agent::AgentConfig;
    use crate::agents::AgentRegistry;
    use crate::dispatch::ToolRegistry;
    use crate::subagent::SubagentManager;
    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::provider::{
        LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
    };
    use archon_llm::streaming::StreamEvent;

    struct MockLlmProvider;

    #[async_trait::async_trait]
    impl LlmProvider for MockLlmProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![]
        }

        fn supports_feature(&self, _: ProviderFeature) -> bool {
            false
        }

        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            unimplemented!()
        }
    }

    fn test_executor(cap: usize) -> Arc<AgentSubagentExecutor> {
        test_executor_with_sink(cap, None)
    }

    pub(super) fn test_executor_with_sink(
        cap: usize,
        activity_sink: Option<Arc<dyn archon_observability::AgentActivitySink>>,
    ) -> Arc<AgentSubagentExecutor> {
        let agent_config = AgentConfig {
            max_subagent_concurrency: cap,
            activity_sink,
            ..Default::default()
        };
        let project_dir = std::env::temp_dir();
        Arc::new(AgentSubagentExecutor::new(
            Arc::new(MockLlmProvider),
            ToolRegistry::new(),
            Arc::new(tokio::sync::Mutex::new(SubagentManager::new(cap))),
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(&project_dir))),
            None,
            None,
            project_dir,
            "test-session".into(),
            "claude-sonnet-4-6".into(),
            vec![],
            Arc::new(tokio::sync::Mutex::new("default".to_string())),
            Arc::new(agent_config),
            Arc::new(IdentityProvider::new(
                IdentityMode::Clean,
                "test-session".into(),
                String::new(),
                String::new(),
            )),
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subagent_capacity_waits_for_slot_instead_of_erroring() {
        let executor = test_executor(2);
        let cancel = tokio_util::sync::CancellationToken::new();
        let first = executor.acquire_subagent_capacity(&cancel).await.unwrap();
        let second = executor.acquire_subagent_capacity(&cancel).await.unwrap();
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired_in_task = Arc::clone(&acquired);
        let queued_executor = Arc::clone(&executor);
        let queued_cancel = cancel.clone();

        let queued = tokio::spawn(async move {
            let _permit = queued_executor
                .acquire_subagent_capacity(&queued_cancel)
                .await
                .expect("queued capacity acquire should wait and then succeed");
            acquired_in_task.store(true, Ordering::SeqCst);
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            !acquired.load(Ordering::SeqCst),
            "overflow acquire must queue while all permits are held"
        );

        drop(first);
        queued.await.unwrap();
        assert!(
            acquired.load(Ordering::SeqCst),
            "queued acquire should succeed after a permit is released"
        );
        drop(second);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn subagent_capacity_queues_batch_without_exceeding_cap() {
        let executor = test_executor(2);
        let current = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..6 {
            let executor = Arc::clone(&executor);
            let current = Arc::clone(&current);
            let peak = Arc::clone(&peak);
            let completed = Arc::clone(&completed);
            tasks.push(tokio::spawn(async move {
                let cancel = tokio_util::sync::CancellationToken::new();
                let _permit = executor.acquire_subagent_capacity(&cancel).await.unwrap();
                let active = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                completed.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(completed.load(Ordering::SeqCst), 6);
        assert!(
            peak.load(Ordering::SeqCst) <= 2,
            "peak active subagents should not exceed configured cap"
        );
    }

    pub(super) fn queued_request() -> SubagentRequest {
        SubagentRequest {
            prompt: "queued".to_string(),
            model: None,
            allowed_tools: Vec::new(),
            max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
            timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            provider_env: None,
        }
    }

    /// Issue 288: with every slot in use, a queued call's dispatch clocks do
    /// not run while it waits; its run time starts when it takes the slot.
    #[tokio::test(start_paused = true)]
    async fn a_queued_calls_dispatch_clock_starts_when_it_takes_the_slot() {
        use archon_tools::subagent_dispatch_clock::{DispatchClock, scope_session};
        let executor = test_executor(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let held = executor.acquire_subagent_capacity(&cancel).await.unwrap();
        let clock = DispatchClock::new();
        let queued = {
            let executor = Arc::clone(&executor);
            let clock = Arc::clone(&clock);
            tokio::spawn(scope_session("queued", vec![clock], async move {
                let mut row = executor.activity_row("queued");
                let permit = executor
                    .acquire_subagent_slot("queued", &queued_request(), &cancel, &mut row)
                    .await
                    .expect("the queued call takes the slot once it is free");
                (permit, tokio::time::Instant::now())
            }))
        };
        tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
        assert!(clock.waiting_for_slot(), "the call is still queued");
        assert_eq!(
            clock.elapsed(),
            std::time::Duration::ZERO,
            "an hour in the queue is not run time"
        );
        drop(held);
        let (_permit, acquired) = queued.await.unwrap();
        assert!(!clock.waiting_for_slot());
        let run = std::time::Duration::from_secs(30);
        tokio::time::sleep_until(acquired + run).await;
        assert_eq!(clock.elapsed(), run, "the clock started at acquisition");
    }

    /// Issue 288: a free slot is taken at once, and that is when the call's
    /// clocks start: the time it spent before reaching the executor is not
    /// run time.
    #[tokio::test(start_paused = true)]
    async fn an_immediately_admitted_calls_clock_starts_at_the_slot() {
        use archon_tools::subagent_dispatch_clock::{DispatchClock, scope_session};
        let executor = test_executor(1);
        let clock = DispatchClock::new();
        tokio::time::sleep(std::time::Duration::from_secs(900)).await;
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut row = executor.activity_row("free");
        let _permit = scope_session("free", vec![Arc::clone(&clock)], async {
            executor
                .acquire_subagent_slot("free", &queued_request(), &cancel, &mut row)
                .await
        })
        .await
        .expect("a free slot");
        assert!(clock.is_admitted(), "taking the slot is reported");
        assert_eq!(
            clock.elapsed(),
            std::time::Duration::ZERO,
            "900s before the slot is not run time"
        );
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        assert_eq!(clock.elapsed(), std::time::Duration::from_secs(30));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_subagent_capacity_acquire_is_cancellable() {
        let executor = test_executor(1);
        let holder_cancel = tokio_util::sync::CancellationToken::new();
        let _held = executor
            .acquire_subagent_capacity(&holder_cancel)
            .await
            .unwrap();
        let queued_cancel = tokio_util::sync::CancellationToken::new();
        let queued_executor = Arc::clone(&executor);
        let queued_cancel_for_task = queued_cancel.clone();

        let queued = tokio::spawn(async move {
            queued_executor
                .acquire_subagent_capacity(&queued_cancel_for_task)
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        queued_cancel.cancel();
        let err = queued
            .await
            .expect("queued acquire task should not panic")
            .expect_err("queued acquire should return cancellation");
        assert!(
            err.to_string().contains("subagent cancelled"),
            "unexpected queued acquire error: {err}"
        );
    }
}

#[cfg(test)]
#[path = "run_followup_tests.rs"]
mod followup_tests;
