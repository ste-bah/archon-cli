use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn run_subagent_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let _capacity_permit = self.acquire_subagent_capacity(&cancel).await?;
        let ids = self.register_subagent_run(&subagent_id, &request).await?;
        let result = self
            .run_registered_subagent_to_completion(&ids, request, ctx, cancel)
            .await;
        self.on_inner_complete(ids.cache_id, result.clone().map_err(|err| err.to_string()))
            .await;
        result
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
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.fire_subagent_start_hooks(&ids.manager_id, &request, ctx.nested)
            .await;
        let prepared = self
            .prepare_subagent_run(&ids.manager_id, &request, &ctx)
            .await?;
        let runner = self
            .build_subagent_runner(ids, &request, &ctx, &prepared, &cancel)
            .await?;
        let activity_model = runner.model().to_string();

        self.emit_subagent_started(
            &ids.cache_id,
            &prepared.activity_agent_type,
            &activity_model,
        );
        let runner_result = runner.run(&request.prompt).await;
        let inner_result = runner_result.map_err(|e| format!("Subagent failed: {e}"));
        self.emit_subagent_finished(
            &ids.cache_id,
            &prepared.activity_agent_type,
            &activity_model,
            &inner_result,
        );

        inner_result.map_err(ExecutorError::Internal)
    }
}

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
        let mut agent_config = AgentConfig::default();
        agent_config.max_subagent_concurrency = cap;
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
            Arc::new(tokio::sync::Mutex::new(None)),
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
