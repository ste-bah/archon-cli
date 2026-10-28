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
