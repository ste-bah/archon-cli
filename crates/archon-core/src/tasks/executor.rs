use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::tasks::events::EventBus;
use crate::tasks::metrics::MetricsRegistry;
use crate::tasks::models::{
    ResourceSample, Task, TaskError, TaskEvent, TaskEventKind, TaskId,
    TaskResultRef, TaskState,
};
use crate::tasks::queue::TaskQueue;
use crate::tasks::store::TaskStateStore;

/// Handle used to cancel a running task. Holds the cancellation token
/// and the JoinHandle for the spawned task so it can be aborted after
/// a grace period.
pub struct CancelHandle {
    pub token: CancellationToken,
    pub join: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Trait abstracting agent invocation. Production impl wraps BackgroundAgents.
/// Tests supply a mock.
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    async fn execute(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<String, TaskError>;
}

/// Central task execution pump.
///
/// Polls per-agent queues, spawns per-task tokio tasks, manages lifecycle,
/// emits events, samples resources, and updates metrics.
pub struct TaskExecutor {
    queue: Arc<dyn TaskQueue>,
    store: Arc<dyn TaskStateStore>,
    event_bus: Arc<EventBus>,
    agent_executor: Arc<dyn AgentExecutor>,
    seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
    tasks: Arc<DashMap<TaskId, Task>>,
    metrics: Arc<MetricsRegistry>,
    poll_interval: Duration,
    shutdown: CancellationToken,
    cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>>,
}

impl TaskExecutor {
    pub fn new(
        queue: Arc<dyn TaskQueue>,
        store: Arc<dyn TaskStateStore>,
        event_bus: Arc<EventBus>,
        agent_executor: Arc<dyn AgentExecutor>,
        seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
        tasks: Arc<DashMap<TaskId, Task>>,
        metrics: Arc<MetricsRegistry>,
        poll_interval: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            queue,
            store,
            event_bus,
            agent_executor,
            seq_counters,
            tasks,
            metrics,
            poll_interval,
            shutdown,
            cancel_handles: Arc::new(DashMap::new()),
        }
    }

    /// Create a TaskExecutor with a pre-existing cancel_handles map
    /// (shared with DefaultTaskService for coordinated cancellation).
    pub fn with_cancel_handles(
        queue: Arc<dyn TaskQueue>,
        store: Arc<dyn TaskStateStore>,
        event_bus: Arc<EventBus>,
        agent_executor: Arc<dyn AgentExecutor>,
        seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
        tasks: Arc<DashMap<TaskId, Task>>,
        metrics: Arc<MetricsRegistry>,
        poll_interval: Duration,
        shutdown: CancellationToken,
        cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>>,
    ) -> Self {
        Self {
            queue,
            store,
            event_bus,
            agent_executor,
            seq_counters,
            tasks,
            metrics,
            poll_interval,
            shutdown,
            cancel_handles,
        }
    }

    /// Access the shared cancel handles map.
    pub fn cancel_handles(&self) -> &Arc<DashMap<TaskId, Arc<CancelHandle>>> {
        &self.cancel_handles
    }

    /// Main poll loop. Iterates agents, dequeues tasks, spawns per-task futures.
    /// Runs until `shutdown` is cancelled.
    ///
    /// Queue operations (`try_dequeue`, `depth`) use `blocking_lock()` internally,
    /// so we run them on a blocking thread to avoid panicking from async context.
    pub async fn run(&self, agents: Vec<String>) {
        loop {
            if self.shutdown.is_cancelled() {
                break;
            }

            for agent in &agents {
                // Update queue depth metric (blocking call).
                let q = self.queue.clone();
                let a = agent.clone();
                let depth = tokio::task::spawn_blocking(move || q.depth(&a))
                    .await
                    .unwrap_or(0);
                self.metrics.set_queue_depth(agent, depth as u64);

                // Try to dequeue a task (blocking call).
                let q = self.queue.clone();
                let a = agent.clone();
                let dequeued = tokio::task::spawn_blocking(move || q.try_dequeue(&a))
                    .await
                    .unwrap_or(None);

                if let Some((task_id, permit)) = dequeued {
                    let queue = self.queue.clone();
                    let store = self.store.clone();
                    let event_bus = self.event_bus.clone();
                    let agent_executor = self.agent_executor.clone();
                    let seq_counters = self.seq_counters.clone();
                    let tasks = self.tasks.clone();
                    let metrics = self.metrics.clone();
                    let agent_name = agent.clone();
                    let cancel_handles = self.cancel_handles.clone();

                    let handle = tokio::spawn(async move {
                        Self::run_task(
                            task_id,
                            agent_name,
                            permit,
                            queue,
                            store,
                            event_bus,
                            agent_executor,
                            seq_counters,
                            tasks,
                            metrics,
                            cancel_handles,
                        )
                        .await;
                    });

                    // Store the JoinHandle in the CancelHandle so abort()
                    // can be called from the watchdog timer.
                    if let Some(ch) = self.cancel_handles.get(&task_id) {
                        let mut guard = ch.join.lock().await;
                        *guard = Some(handle);
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {}
                _ = self.shutdown.cancelled() => { break; }
            }
        }
    }

    #[instrument(skip_all, fields(task_id = %task_id, agent = %agent_name))]
    async fn run_task(
        task_id: TaskId,
        agent_name: String,
        _permit: tokio::sync::OwnedSemaphorePermit,
        _queue: Arc<dyn TaskQueue>,
        store: Arc<dyn TaskStateStore>,
        event_bus: Arc<EventBus>,
        agent_executor: Arc<dyn AgentExecutor>,
        seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
        tasks: Arc<DashMap<TaskId, Task>>,
        metrics: Arc<MetricsRegistry>,
        cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>>,
    ) {
        // Helper to get next seq for this task.
        let next_seq = |counters: &DashMap<TaskId, AtomicU64>| -> u64 {
            counters
                .entry(task_id)
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed)
        };

        // 1. Transition to Running.
        {
            if let Some(mut task) = tasks.get_mut(&task_id) {
                task.state = TaskState::Running;
                task.started_at = Some(Utc::now());
                let _ = store.put_snapshot(&task);
            }
        }

        metrics.inc_started();

        // 2. Create cancel token and register a CancelHandle.
        let cancel_token = CancellationToken::new();
        let cancel_handle = Arc::new(CancelHandle {
            token: cancel_token.clone(),
            join: tokio::sync::Mutex::new(None),
        });
        cancel_handles.insert(task_id, cancel_handle);

        // 3. Emit Started event.
        let started_event = TaskEvent {
            task_id,
            seq: next_seq(&seq_counters),
            kind: TaskEventKind::Started,
            payload: serde_json::json!({}),
            at: Utc::now(),
        };
        event_bus.broadcast(task_id, started_event);

        // 4. Get input from task.
        let input = tasks
            .get(&task_id)
            .map(|t| t.input.clone())
            .unwrap_or(serde_json::json!({}));

        // 5. Spawn resource sampler.
        let sampler_cancel = cancel_token.clone();
        let sampler_tasks = tasks.clone();
        let sampler_task_id = task_id;
        let sampler_handle = tokio::spawn(async move {
            let mut sys = sysinfo::System::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    _ = sampler_cancel.cancelled() => { break; }
                }
                sys.refresh_cpu_usage();
                sys.refresh_memory();

                // Get current process stats.
                let pid = sysinfo::get_current_pid();
                if let Ok(pid) = pid {
                    sys.refresh_processes(
                        sysinfo::ProcessesToUpdate::Some(&[pid]),
                        true,
                    );
                    if let Some(proc_info) = sys.process(pid) {
                        let cpu_ms = (proc_info.cpu_usage() * 10.0) as u64;
                        let rss_bytes = proc_info.memory();
                        if let Some(mut task) = sampler_tasks.get_mut(&sampler_task_id) {
                            task.resource_usage =
                                Some(ResourceSample { cpu_ms, rss_bytes });
                        }
                    }
                }
            }
        });

        // 6. Execute agent with cancellation support via tokio::select!
        let cancelled_token = cancel_token.clone();
        let execute_result = tokio::select! {
            res = agent_executor.execute(&agent_name, input, cancel_token.clone()) => {
                Some(res)
            }
            _ = cancelled_token.cancelled() => {
                None // Cancelled path
            }
        };

        // 7. Stop resource sampler.
        cancel_token.cancel();
        let _ = sampler_handle.await;

        // 8. Persist terminal state and emit events.
        match execute_result {
            Some(Ok(output)) => {
                if let Some(mut task) = tasks.get_mut(&task_id) {
                    task.state = TaskState::Finished;
                    task.finished_at = Some(Utc::now());
                    task.result_ref = Some(TaskResultRef {
                        inline: Some(output),
                        file_path: None,
                        streaming_handle: None,
                    });
                    let _ = store.put_snapshot(&task);
                }
                metrics.inc_finished();

                let finished_event = TaskEvent {
                    task_id,
                    seq: next_seq(&seq_counters),
                    kind: TaskEventKind::Finished,
                    payload: serde_json::json!({}),
                    at: Utc::now(),
                };
                event_bus.broadcast(task_id, finished_event);
            }
            Some(Err(e)) => {
                if let Some(mut task) = tasks.get_mut(&task_id) {
                    task.state = TaskState::Failed;
                    task.finished_at = Some(Utc::now());
                    task.error = Some(format!("{}", e));
                    let _ = store.put_snapshot(&task);
                }
                metrics.inc_failed();

                let failed_event = TaskEvent {
                    task_id,
                    seq: next_seq(&seq_counters),
                    kind: TaskEventKind::Failed,
                    payload: serde_json::json!({"error": format!("{}", e)}),
                    at: Utc::now(),
                };
                event_bus.broadcast(task_id, failed_event);
            }
            None => {
                // Cancelled path.
                if let Some(mut task) = tasks.get_mut(&task_id) {
                    task.state = TaskState::Cancelled;
                    task.finished_at = Some(Utc::now());
                    let _ = store.put_snapshot(&task);
                }
                metrics.inc_cancelled();

                let cancelled_event = TaskEvent {
                    task_id,
                    seq: next_seq(&seq_counters),
                    kind: TaskEventKind::Cancelled,
                    payload: serde_json::json!({}),
                    at: Utc::now(),
                };
                event_bus.broadcast(task_id, cancelled_event);
            }
        }

        // 9. Remove the cancel handle — task is done.
        cancel_handles.remove(&task_id);
    }
}
