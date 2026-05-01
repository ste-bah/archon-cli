//! Tests for TaskExecutor poll loop + MetricsRegistry (TASK-AGS-206).

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use archon_core::tasks::events::EventBus;
use archon_core::tasks::executor::{AgentExecutor, TaskExecutor};
use archon_core::tasks::metrics::MetricsRegistry;
use archon_core::tasks::models::{Task, TaskError, TaskEventKind, TaskId, TaskState};
use archon_core::tasks::queue::{PerAgentTaskQueue, QueueConfig, TaskQueue};
use archon_core::tasks::store::{InMemoryTaskStateStore, TaskStateStore};

// ---------------------------------------------------------------------------
// MockAgentExecutor
// ---------------------------------------------------------------------------

struct MockAgentExecutor {
    delay: Duration,
    invocations: Arc<Mutex<Vec<String>>>,
    fail_agents: HashSet<String>,
    concurrent_count: Arc<AtomicU64>,
    max_concurrent_seen: Arc<AtomicU64>,
}

impl MockAgentExecutor {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            invocations: Arc::new(Mutex::new(Vec::new())),
            fail_agents: HashSet::new(),
            concurrent_count: Arc::new(AtomicU64::new(0)),
            max_concurrent_seen: Arc::new(AtomicU64::new(0)),
        }
    }

    #[allow(dead_code)] // Test helper kept for future use; flagged as dead by current test set.
    fn with_fail_agents(mut self, agents: HashSet<String>) -> Self {
        self.fail_agents = agents;
        self
    }
}

#[async_trait]
impl AgentExecutor for MockAgentExecutor {
    async fn execute(
        &self,
        agent_name: &str,
        _input: serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<String, TaskError> {
        // Track concurrency.
        let prev = self.concurrent_count.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        // Update max seen.
        self.max_concurrent_seen
            .fetch_max(current, Ordering::SeqCst);

        // Record invocation.
        {
            let mut inv = self.invocations.lock().unwrap();
            inv.push(agent_name.to_string());
        }

        // Simulate work.
        tokio::time::sleep(self.delay).await;

        // Decrement concurrent count.
        self.concurrent_count.fetch_sub(1, Ordering::SeqCst);

        if self.fail_agents.contains(agent_name) {
            Err(TaskError::InvalidState)
        } else {
            Ok(format!("result-from-{}", agent_name))
        }
    }
}

// ---------------------------------------------------------------------------
// Test setup helper
// ---------------------------------------------------------------------------

struct TestHarness {
    queue: Arc<PerAgentTaskQueue>,
    store: Arc<InMemoryTaskStateStore>,
    event_bus: Arc<EventBus>,
    tasks: Arc<DashMap<TaskId, Task>>,
    seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
    metrics: Arc<MetricsRegistry>,
}

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

/// Setup harness. Safe to call from any context — the queue uses
/// `std::sync::Mutex` internally (no `.await` inside critical sections).
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
    };
    (harness, task_ids)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_executor_dequeues_and_invokes_agent() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, _task_ids) = setup_harness(config, "test-agent", 5).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(10)));

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(10),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for tasks to complete.
    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    let invocations = mock.invocations.lock().unwrap();
    assert_eq!(
        invocations.len(),
        5,
        "expected 5 invocations, got {}",
        invocations.len()
    );
    assert!(invocations.iter().all(|name| name == "test-agent"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_executor_respects_semaphore() {
    let config = QueueConfig {
        max_concurrent: 2,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(300),
    };
    let (harness, _task_ids) = setup_harness(config, "test-agent", 10).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(100)));

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(20),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for all tasks to complete (10 tasks, max 2 concurrent, 100ms each = ~500ms).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    let max_seen = mock.max_concurrent_seen.load(Ordering::SeqCst);
    assert!(
        max_seen <= 2,
        "expected max concurrent <= 2, got {}",
        max_seen
    );

    let invocations = mock.invocations.lock().unwrap();
    assert_eq!(
        invocations.len(),
        10,
        "expected 10 invocations, got {}",
        invocations.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_executor_forwards_events_with_monotonic_seq() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(10)));

    let task_id = task_ids[0];

    // Subscribe BEFORE running the executor so we catch the events.
    let mut rx = harness.event_bus.subscribe(task_id);

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(10),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Collect events.
    let mut events = Vec::new();
    let timeout = Duration::from_secs(5);
    while let Ok(Some(evt)) = tokio::time::timeout(timeout, rx.next()).await {
        events.push(evt);
        if events.len() >= 2 {
            break;
        }
    }

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    assert_eq!(
        events.len(),
        2,
        "expected 2 events (Started, Finished), got {}",
        events.len()
    );
    assert_eq!(events[0].kind, TaskEventKind::Started);
    assert_eq!(events[1].kind, TaskEventKind::Finished);
    assert_eq!(events[0].seq, 0);
    assert_eq!(events[1].seq, 1);
    assert!(
        events[0].seq < events[1].seq,
        "seq must be monotonically increasing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_executor_persists_terminal_state() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(10)));

    let task_id = task_ids[0];

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(10),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    // Verify store has the terminal state.
    let snapshot = harness.store.get_snapshot(task_id).unwrap();
    assert_eq!(
        snapshot.state,
        TaskState::Finished,
        "expected Finished state in store, got {:?}",
        snapshot.state
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_executor_samples_resource_usage() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 100,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(30),
    };
    // Single task with 600ms delay so the resource sampler (500ms interval) fires at least once.
    let (harness, task_ids) = setup_harness(config, "test-agent", 1).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(600)));

    let task_id = task_ids[0];

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(10),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Wait for task to complete (600ms agent delay + sampler overhead).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    // Check that resource_usage was populated.
    let task = harness
        .tasks
        .get(&task_id)
        .expect("task should still be in map");
    let usage = task
        .resource_usage
        .as_ref()
        .expect("resource_usage should be Some");
    assert!(
        usage.rss_bytes > 0,
        "expected rss_bytes > 0, got {}",
        usage.rss_bytes
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_metrics_registry_prometheus_format() {
    let metrics = MetricsRegistry::new();

    // Increment counters.
    metrics.inc_started();
    metrics.inc_started();
    metrics.inc_started();
    metrics.inc_finished();
    metrics.inc_finished();
    metrics.inc_failed();

    // Set queue depths.
    metrics.set_queue_depth("agent-b", 5);
    metrics.set_queue_depth("agent-a", 3);

    let output = metrics.export_prometheus();

    assert!(output.contains("tasks_started_total 3\n"));
    assert!(output.contains("tasks_finished_total 2\n"));
    assert!(output.contains("tasks_failed_total 1\n"));
    assert!(output.contains("queue_depth{agent=\"agent-a\"} 3\n"));
    assert!(output.contains("queue_depth{agent=\"agent-b\"} 5\n"));

    // Verify agent-a appears before agent-b (sorted).
    let a_pos = output.find("agent-a").unwrap();
    let b_pos = output.find("agent-b").unwrap();
    assert!(a_pos < b_pos, "agents should be sorted alphabetically");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_executor_100_concurrent_tasks() {
    let config = QueueConfig {
        max_concurrent: 10,
        queue_capacity: 200,
        burst_capacity: 0,
        burst_threshold: Duration::from_secs(300),
    };
    let (harness, _task_ids) = setup_harness(config, "test-agent", 100).await;
    let mock = Arc::new(MockAgentExecutor::new(Duration::from_millis(10)));

    let shutdown = CancellationToken::new();
    let executor = TaskExecutor::new(
        harness.queue.clone(),
        harness.store.clone(),
        harness.event_bus.clone(),
        mock.clone() as Arc<dyn AgentExecutor>,
        harness.seq_counters.clone(),
        harness.tasks.clone(),
        harness.metrics.clone(),
        Duration::from_millis(20),
        shutdown.clone(),
    );

    let handle = tokio::spawn(async move {
        executor.run(vec!["test-agent".to_string()]).await;
    });

    // Timeout: 30 seconds. 100 tasks at max 10 concurrent, 10ms each = ~100ms ideal,
    // but with poll intervals and overhead, give generous timeout.
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let finished = harness.metrics.finished_total();
            if finished >= 100 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    assert!(
        result.is_ok(),
        "100 tasks should complete within 30 seconds (deadlock?)"
    );

    let invocations = mock.invocations.lock().unwrap();
    assert_eq!(
        invocations.len(),
        100,
        "expected 100 invocations, got {}",
        invocations.len()
    );

    let max_seen = mock.max_concurrent_seen.load(Ordering::SeqCst);
    assert!(
        max_seen <= 10,
        "expected max concurrent <= 10, got {}",
        max_seen
    );
}
