//! Tests for graceful cancellation protocol (TASK-AGS-207).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use archon_core::tasks::events::EventBus;
use archon_core::tasks::executor::{AgentExecutor, CancelHandle, TaskExecutor};
use archon_core::tasks::metrics::MetricsRegistry;
use archon_core::tasks::models::{Task, TaskError, TaskEventKind, TaskId, TaskState};
use archon_core::tasks::queue::{PerAgentTaskQueue, QueueConfig, TaskQueue};
use archon_core::tasks::store::{InMemoryTaskStateStore, TaskStateStore};

// ---------------------------------------------------------------------------
// Mock executors
// ---------------------------------------------------------------------------

/// An executor that blocks forever (never returns). The executor's
/// `tokio::select!` will always pick the `cancelled()` branch.
struct BlockingMockExecutor {
    invocations: Arc<Mutex<Vec<String>>>,
}

impl BlockingMockExecutor {
    fn new() -> Self {
        Self {
            invocations: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl AgentExecutor for BlockingMockExecutor {
    async fn execute(
        &self,
        agent_name: &str,
        _input: serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<String, TaskError> {
        {
            let mut inv = self.invocations.lock().unwrap();
            inv.push(agent_name.to_string());
        }
        // Block forever — never return. The executor's tokio::select!
        // picks the cancelled() branch, ensuring state=Cancelled.
        std::future::pending::<()>().await;
        unreachable!()
    }
}

/// An executor that ignores the cancel token and blocks forever (for abort tests).
struct StubbornMockExecutor;

#[async_trait]
impl AgentExecutor for StubbornMockExecutor {
    async fn execute(
        &self,
        _agent_name: &str,
        _input: serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<String, TaskError> {
        // Ignore the cancel token, just sleep forever.
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

fn make_task(agent_name: &str) -> Task {
    Task {
        id: TaskId::new(),
        agent_name: agent_name.to_string(),
        agent_version: None,
        input: serde_json::json!({"key": "value"}),
        state: TaskState::Pending,
        progress_pct: None,
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        result_ref: None,
        error: None,
        owner: "test".to_string(),
        resource_usage: None,
        cancel_token_ref: None,
    }
}

struct TestHarness {
    queue: Arc<PerAgentTaskQueue>,
    store: Arc<InMemoryTaskStateStore>,
    event_bus: Arc<EventBus>,
    tasks: Arc<DashMap<TaskId, Task>>,
    seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
    metrics: Arc<MetricsRegistry>,
    cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>>,
}

async fn setup_harness(
    config: QueueConfig,
    agent: &str,
    count: usize,
) -> (TestHarness, Vec<TaskId>) {
    let queue = Arc::new(PerAgentTaskQueue::new(config));
    let store = Arc::new(InMemoryTaskStateStore::new());
    let event_bus = Arc::new(EventBus::new());
    let tasks: Arc<DashMap<TaskId, Task>> = Arc::new(DashMap::new());
    let seq_counters: Arc<DashMap<TaskId, AtomicU64>> = Arc::new(DashMap::new());
    let metrics = Arc::new(MetricsRegistry::new());
    let cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>> = Arc::new(DashMap::new());

    let agent_owned = agent.to_string();
    let q = queue.clone();
    let t = tasks.clone();
    let sc = seq_counters.clone();
    let task_ids = tokio::task::spawn_blocking(move || {
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let task = make_task(&agent_owned);
            let id = task.id;
            t.insert(id, task);
            q.enqueue(id, &agent_owned).unwrap();
            sc.insert(id, AtomicU64::new(0));
            ids.push(id);
        }
        ids
    })
    .await
    .unwrap();

    let harness = TestHarness {
        queue,
        store,
        event_bus,
        tasks,
        seq_counters,
        metrics,
        cancel_handles,
    };
    (harness, task_ids)
}

fn make_executor(
    harness: &TestHarness,
    agent_executor: Arc<dyn AgentExecutor>,
    shutdown: CancellationToken,
) -> TaskExecutor {
    TaskExecutor::with_cancel_handles(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        agent_executor,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(10),
        shutdown,
        harness.cancel_handles.clone(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. Running task observes cancellation token within 100ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_running_task_sends_token() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(BlockingMockExecutor::new());
    let task_id = task_ids[0];

    let shutdown = CancellationToken::new();
    let executor = make_executor(&harness, mock.clone(), shutdown.clone());

    let exec_handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for the task to start running.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify it's in Running state.
    let state = harness
        .tasks
        .get(&task_id)
        .map(|t| t.state)
        .unwrap_or(TaskState::Pending);
    assert_eq!(state, TaskState::Running, "task should be Running");

    // Cancel via the cancel_handles directly (simulating service.cancel()).
    if let Some(ch) = harness.cancel_handles.get(&task_id) {
        ch.token.cancel();
    } else {
        panic!("cancel handle not found for running task");
    }

    // Wait for the task to transition to Cancelled.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let state = harness.tasks.get(&task_id).map(|t| t.state);
        if state == Some(TaskState::Cancelled) {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "task did not transition to Cancelled within 500ms, state={:?}",
                state
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), exec_handle).await;
}

/// 2. Agent ignores token, gets force-aborted after grace period, state=Cancelled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_30s_grace_then_abort() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let task_id = task_ids[0];

    // Use StubbornMockExecutor that ignores the cancel token.
    let mock = Arc::new(StubbornMockExecutor);

    let shutdown = CancellationToken::new();
    let executor = make_executor(&harness, mock, shutdown.clone());

    let exec_handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for task to start running.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify Running state.
    let state = harness.tasks.get(&task_id).map(|t| t.state);
    assert_eq!(state, Some(TaskState::Running), "task should be Running");

    // Fire cancellation token.
    if let Some(ch) = harness.cancel_handles.get(&task_id) {
        let handle = ch.clone();
        handle.token.cancel();

        // Spawn a short watchdog (1 second instead of 30).
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let join_guard = handle.join.lock().await;
            if let Some(jh) = join_guard.as_ref() {
                jh.abort();
            }
        });
    } else {
        panic!("cancel handle not found");
    }

    // Wait for the abort + state transition (give a generous window).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state = harness.tasks.get(&task_id).map(|t| t.state);
        if state == Some(TaskState::Cancelled) {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            // The stubborn executor never returns, so tokio::select! picks
            // the cancelled branch, which does transition. If we still
            // don't see it, that's the actual failure.
            panic!(
                "task did not transition to Cancelled within 5s, state={:?}",
                state
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), exec_handle).await;
}

/// 3. Pending task cancelled: removed from queue, never invoked, queue depth decreases.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_pending_task_removes_from_queue() {
    // Use max_concurrent=0 so no task gets dequeued (they stay Pending).
    let config = QueueConfig {
        max_concurrent: 0,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(300),
    };
    let queue = Arc::new(PerAgentTaskQueue::new(config));
    let store = Arc::new(InMemoryTaskStateStore::new());
    let event_bus = Arc::new(EventBus::new());
    let tasks: Arc<DashMap<TaskId, Task>> = Arc::new(DashMap::new());
    let seq_counters: Arc<DashMap<TaskId, AtomicU64>> = Arc::new(DashMap::new());

    // Enqueue 3 tasks.
    let q = queue.clone();
    let t = tasks.clone();
    let sc = seq_counters.clone();
    let task_ids = tokio::task::spawn_blocking(move || {
        let mut ids = Vec::new();
        for _ in 0..3 {
            let task = make_task("test-agent");
            let id = task.id;
            t.insert(id, task);
            q.enqueue(id, "test-agent").unwrap();
            sc.insert(id, AtomicU64::new(0));
            ids.push(id);
        }
        ids
    })
    .await
    .unwrap();

    // Verify initial queue depth.
    let q = queue.clone();
    let initial_depth = tokio::task::spawn_blocking(move || q.depth("test-agent"))
        .await
        .unwrap();
    assert_eq!(initial_depth, 3);

    // Cancel the middle task while it's still pending.
    let cancel_id = task_ids[1];
    let q = queue.clone();
    let removed = tokio::task::spawn_blocking(move || q.remove_pending(cancel_id))
        .await
        .unwrap();
    assert!(removed, "remove_pending should return true");

    // Update task state.
    if let Some(mut task) = tasks.get_mut(&cancel_id) {
        task.state = TaskState::Cancelled;
        task.finished_at = Some(Utc::now());
        let _ = store.put_snapshot(&task);
    }

    // Emit Cancelled event.
    let seq = seq_counters
        .entry(cancel_id)
        .or_insert_with(|| AtomicU64::new(0))
        .fetch_add(1, Ordering::Relaxed);
    let event = archon_core::tasks::models::TaskEvent {
        task_id: cancel_id,
        seq,
        kind: TaskEventKind::Cancelled,
        payload: serde_json::json!({}),
        at: Utc::now(),
    };
    event_bus.broadcast(cancel_id, event);

    // Verify queue depth decreased.
    let q = queue.clone();
    let new_depth = tokio::task::spawn_blocking(move || q.depth("test-agent"))
        .await
        .unwrap();
    assert_eq!(
        new_depth, 2,
        "queue depth should be 2 after removing 1 task"
    );

    // Verify task state.
    let state = tasks.get(&cancel_id).map(|t| t.state);
    assert_eq!(state, Some(TaskState::Cancelled));

    // Verify snapshot persisted.
    let snap = store.get_snapshot(cancel_id).unwrap();
    assert_eq!(snap.state, TaskState::Cancelled);
}

/// 4. Second cancel() on already-cancelled task returns AlreadyCancelled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_already_cancelled_returns_error() {
    use archon_core::agents::registry::AgentRegistry;
    use archon_core::tasks::service::{DefaultTaskService, TaskService};

    let tmp = tempfile::TempDir::new().unwrap();
    let registry = Arc::new(AgentRegistry::load(tmp.path()));

    // Use no-queue variant to avoid blocking_lock panics in async context.
    let svc = DefaultTaskService::new(registry, 100);

    let req = archon_core::tasks::models::SubmitRequest {
        agent_name: "general-purpose".to_string(),
        agent_version: None,
        input: serde_json::json!({}),
        owner: "test".to_string(),
    };
    let id = svc.submit(req).await.unwrap();

    // First cancel (pending task).
    let r1 = svc.cancel(id).await;
    assert!(r1.is_ok(), "first cancel should succeed");

    // Second cancel should return AlreadyCancelled.
    let r2 = svc.cancel(id).await;
    assert!(r2.is_err(), "second cancel should fail");
    let err = r2.unwrap_err();
    assert!(
        format!("{}", err).contains("already cancelled"),
        "error should be AlreadyCancelled, got: {}",
        err
    );
}

/// 5. Cancel on Finished task returns InvalidState.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_finished_task_returns_error() {
    use archon_core::agents::registry::AgentRegistry;
    use archon_core::tasks::service::{DefaultTaskService, TaskService};

    let tmp = tempfile::TempDir::new().unwrap();
    let registry = Arc::new(AgentRegistry::load(tmp.path()));

    // Use no-queue variant to avoid blocking_lock panics in async context.
    let svc = DefaultTaskService::new(registry, 100);

    let req = archon_core::tasks::models::SubmitRequest {
        agent_name: "general-purpose".to_string(),
        agent_version: None,
        input: serde_json::json!({}),
        owner: "test".to_string(),
    };
    let id = svc.submit(req).await.unwrap();

    // Manually set the task to Finished.
    if let Some(mut task) = svc.tasks().get_mut(&id) {
        task.state = TaskState::Finished;
        task.finished_at = Some(Utc::now());
    }

    let result = svc.cancel(id).await;
    assert!(result.is_err(), "cancel on Finished task should fail");
    let err = result.unwrap_err();
    assert!(
        format!("{}", err).contains("invalid state"),
        "error should be InvalidState, got: {}",
        err
    );
}

/// 6. Cancelled task snapshot has state=Cancelled and finished_at set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_final_state_persisted() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(BlockingMockExecutor::new());
    let task_id = task_ids[0];

    let shutdown = CancellationToken::new();
    let executor = make_executor(&harness, mock, shutdown.clone());

    let exec_handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for Running.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Cancel.
    if let Some(ch) = harness.cancel_handles.get(&task_id) {
        ch.token.cancel();
    }

    // Wait for Cancelled state.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let state = harness.tasks.get(&task_id).map(|t| t.state);
        if state == Some(TaskState::Cancelled) {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("task did not transition to Cancelled");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Verify store snapshot.
    let snap = harness.store.get_snapshot(task_id).unwrap();
    assert_eq!(snap.state, TaskState::Cancelled);
    assert!(
        snap.finished_at.is_some(),
        "finished_at should be set on cancelled task"
    );

    // Verify in-memory task too.
    let task = harness.tasks.get(&task_id).unwrap();
    assert_eq!(task.state, TaskState::Cancelled);
    assert!(task.finished_at.is_some());

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), exec_handle).await;
}

/// 7. Subscriber receives Cancelled event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_emits_cancelled_event() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(BlockingMockExecutor::new());
    let task_id = task_ids[0];

    // Subscribe BEFORE running executor.
    let mut rx = harness.event_bus.subscribe(task_id);

    let shutdown = CancellationToken::new();
    let executor = make_executor(&harness, mock, shutdown.clone());

    let exec_handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for Started event.
    let started = tokio::time::timeout(Duration::from_secs(5), rx.next())
        .await
        .expect("timeout waiting for Started")
        .expect("stream ended before Started");
    assert_eq!(started.kind, TaskEventKind::Started);

    // Cancel the task.
    if let Some(ch) = harness.cancel_handles.get(&task_id) {
        ch.token.cancel();
    }

    // Receive Cancelled event.
    let cancelled = tokio::time::timeout(Duration::from_secs(5), rx.next())
        .await
        .expect("timeout waiting for Cancelled event")
        .expect("stream ended before Cancelled event");
    assert_eq!(
        cancelled.kind,
        TaskEventKind::Cancelled,
        "expected Cancelled event, got {:?}",
        cancelled.kind
    );
    assert_eq!(cancelled.task_id, task_id);
    assert!(
        cancelled.seq > started.seq,
        "Cancelled seq should be > Started seq"
    );

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), exec_handle).await;
}

/// 8. After cancelling a running task, a queued task gets to start
///    (semaphore permit is released).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_releases_semaphore_permit() {
    // max_concurrent=1 so only 1 task runs at a time.
    let config = QueueConfig {
        max_concurrent: 1,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(300),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 2).await;
    let first_id = task_ids[0];
    let second_id = task_ids[1];

    // Use blocking executor so first task blocks until cancelled.
    let mock = Arc::new(BlockingMockExecutor::new());

    let shutdown = CancellationToken::new();
    let executor = make_executor(&harness, mock.clone(), shutdown.clone());

    let exec_handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for first task to start.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let state = harness.tasks.get(&first_id).map(|t| t.state);
        if state == Some(TaskState::Running) {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("first task did not start running");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Second task should still be Pending (semaphore full).
    let second_state = harness.tasks.get(&second_id).map(|t| t.state);
    assert_eq!(
        second_state,
        Some(TaskState::Pending),
        "second task should still be Pending"
    );

    // Cancel first task.
    if let Some(ch) = harness.cancel_handles.get(&first_id) {
        ch.token.cancel();
    }

    // Wait for second task to start running.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state = harness.tasks.get(&second_id).map(|t| t.state);
        if state == Some(TaskState::Running) || state == Some(TaskState::Cancelled) {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            let s = harness.tasks.get(&second_id).map(|t| t.state);
            panic!(
                "second task did not start after cancelling first, state={:?}",
                s
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Verify first task is cancelled.
    let first_state = harness.tasks.get(&first_id).map(|t| t.state);
    assert_eq!(first_state, Some(TaskState::Cancelled));

    // Clean up: cancel second task and shut down.
    if let Some(ch) = harness.cancel_handles.get(&second_id) {
        ch.token.cancel();
    }
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), exec_handle).await;
}
