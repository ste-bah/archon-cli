use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use tokio_stream::Stream;

use crate::agents::registry::AgentRegistry;
use crate::tasks::events::EventBus;
use crate::tasks::executor::CancelHandle;
use crate::tasks::models::{
    SubmitRequest, Task, TaskError, TaskEvent, TaskEventKind, TaskFilter, TaskId, TaskResultStream,
    TaskSnapshot, TaskState,
};
use crate::tasks::queue::TaskQueue;
use crate::tasks::store::{InMemoryTaskStateStore, TaskStateStore};

/// Core async task service trait (TECH-AGS-ASYNC L329-337).
#[async_trait]
pub trait TaskService: Send + Sync {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError>;
    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError>;
    async fn result(&self, id: TaskId, stream: bool) -> Result<TaskResultStream, TaskError>;
    async fn cancel(&self, id: TaskId) -> Result<(), TaskError>;
    async fn subscribe_events(
        &self,
        id: TaskId,
        from_seq: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError>;
    async fn list(&self, filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError>;
}

/// Default implementation backed by in-memory DashMap storage.
///
/// The submit path is live; other methods return `Unimplemented`
/// until their respective tasks land (AGS-202 through AGS-207).
pub struct DefaultTaskService {
    registry: Arc<AgentRegistry>,
    tasks: Arc<DashMap<TaskId, Task>>,
    seq_counters: Arc<DashMap<TaskId, AtomicU64>>,
    max_queue_size: usize,
    queue: Option<Arc<dyn TaskQueue>>,
    store: Arc<dyn TaskStateStore>,
    event_bus: Arc<EventBus>,
    cancel_handles: Arc<DashMap<TaskId, Arc<CancelHandle>>>,
    /// Grace period before a running task is force-aborted after
    /// cancellation token fires. Default: 30 seconds.
    grace_period: Duration,
}

impl DefaultTaskService {
    pub fn new(registry: Arc<AgentRegistry>, max_queue_size: usize) -> Self {
        Self {
            registry,
            tasks: Arc::new(DashMap::new()),
            seq_counters: Arc::new(DashMap::new()),
            max_queue_size,
            queue: None,
            store: Arc::new(InMemoryTaskStateStore::new()),
            event_bus: Arc::new(EventBus::new()),
            cancel_handles: Arc::new(DashMap::new()),
            grace_period: Duration::from_secs(30),
        }
    }

    /// Create a service backed by a per-agent bounded queue.
    pub fn with_queue(
        registry: Arc<AgentRegistry>,
        queue: Arc<dyn TaskQueue>,
        max_queue_size: usize,
    ) -> Self {
        Self {
            registry,
            tasks: Arc::new(DashMap::new()),
            seq_counters: Arc::new(DashMap::new()),
            max_queue_size,
            queue: Some(queue),
            store: Arc::new(InMemoryTaskStateStore::new()),
            event_bus: Arc::new(EventBus::new()),
            cancel_handles: Arc::new(DashMap::new()),
            grace_period: Duration::from_secs(30),
        }
    }

    /// Create a service with a custom grace period (for testing).
    pub fn with_queue_and_grace(
        registry: Arc<AgentRegistry>,
        queue: Arc<dyn TaskQueue>,
        max_queue_size: usize,
        grace_period: Duration,
    ) -> Self {
        Self {
            registry,
            tasks: Arc::new(DashMap::new()),
            seq_counters: Arc::new(DashMap::new()),
            max_queue_size,
            queue: Some(queue),
            store: Arc::new(InMemoryTaskStateStore::new()),
            event_bus: Arc::new(EventBus::new()),
            cancel_handles: Arc::new(DashMap::new()),
            grace_period,
        }
    }

    /// Access the shared cancel handles map (used by executor).
    pub fn cancel_handles(&self) -> &Arc<DashMap<TaskId, Arc<CancelHandle>>> {
        &self.cancel_handles
    }

    /// Access the shared tasks map (used by downstream tasks for hot-cache).
    pub fn tasks(&self) -> &Arc<DashMap<TaskId, Task>> {
        &self.tasks
    }

    /// Access the shared seq counters (used by executor + events).
    pub fn seq_counters(&self) -> &Arc<DashMap<TaskId, AtomicU64>> {
        &self.seq_counters
    }

    /// Access the shared event bus (used by executor + events).
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }
}

#[async_trait]
impl TaskService for DefaultTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        // 1. Validate agent exists in registry (in-memory lookup, no I/O).
        let id_for_err = TaskId::new();
        if self.registry.resolve(&req.agent_name).is_none() {
            return Err(TaskError::NotFound(id_for_err));
        }

        // 2. Check queue capacity.
        if self.tasks.len() >= self.max_queue_size {
            return Err(TaskError::QueueFull);
        }

        // 3. Generate TaskId and construct Task.
        let task_id = TaskId::new();
        let task = Task {
            id: task_id,
            agent_name: req.agent_name,
            agent_version: req.agent_version,
            input: req.input,
            state: TaskState::Pending,
            progress_pct: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            result_ref: None,
            error: None,
            owner: req.owner,
            resource_usage: None,
            cancel_token_ref: None,
        };

        // 4. Insert into in-memory store (no I/O).
        self.tasks.insert(task_id, task.clone());

        // 4b. Write snapshot to hot-cache store (TASK-AGS-202).
        let _ = self.store.put_snapshot(&task);

        // 5. Initialize per-task seq counter (REQ-ASYNC-009).
        self.seq_counters.insert(task_id, AtomicU64::new(0));

        // 6. Enqueue into per-agent queue when configured.
        if let Some(ref queue) = self.queue
            && let Err(e) = queue.enqueue(task_id, &task.agent_name) {
                // Roll back the in-memory inserts on queue failure.
                self.tasks.remove(&task_id);
                self.seq_counters.remove(&task_id);
                return Err(e);
            }

        Ok(task_id)
    }

    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        self.store.get_snapshot(id)
    }

    async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        // 1. Get snapshot from store to check state.
        let snap = self.store.get_snapshot(id)?;

        // 2. Must be in a terminal state before results are available.
        if !snap.state.is_terminal() {
            return Err(TaskError::Pending);
        }

        // 3. Look up full task for result_ref.
        let task = self.tasks.get(&id).ok_or(TaskError::NotFound(id))?;

        match &task.result_ref {
            Some(rr) => {
                if let Some(ref inline) = rr.inline {
                    Ok(TaskResultStream::Inline(inline.clone()))
                } else if let Some(ref path) = rr.file_path {
                    Ok(TaskResultStream::File(path.clone()))
                } else {
                    Err(TaskError::NotFound(id))
                }
            }
            None => Err(TaskError::NotFound(id)),
        }
    }

    async fn cancel(&self, id: TaskId) -> Result<(), TaskError> {
        // 1. Get current state.
        let task = self.tasks.get(&id).ok_or(TaskError::NotFound(id))?;
        let state = task.state;
        drop(task); // release DashMap ref before mutation

        match state {
            // Already terminal — error.
            TaskState::Finished | TaskState::Failed | TaskState::Corrupted => {
                Err(TaskError::InvalidState)
            }
            TaskState::Cancelled => Err(TaskError::AlreadyCancelled),

            // Pending: remove from queue, mark cancelled, never invoke.
            TaskState::Pending => {
                if let Some(ref q) = self.queue {
                    q.remove_pending(id);
                }
                if let Some(mut task) = self.tasks.get_mut(&id) {
                    task.state = TaskState::Cancelled;
                    task.finished_at = Some(Utc::now());
                    let _ = self.store.put_snapshot(&task);
                }
                // Emit Cancelled event.
                let seq = self
                    .seq_counters
                    .entry(id)
                    .or_insert_with(|| AtomicU64::new(0))
                    .fetch_add(1, Ordering::Relaxed);
                let event = TaskEvent {
                    task_id: id,
                    seq,
                    kind: TaskEventKind::Cancelled,
                    payload: serde_json::json!({}),
                    at: Utc::now(),
                };
                self.event_bus.broadcast(id, event);
                Ok(())
            }

            // Running: fire cancellation token, spawn grace-period watchdog.
            TaskState::Running => {
                if let Some(handle) = self.cancel_handles.get(&id) {
                    let handle = handle.clone();
                    handle.token.cancel();

                    // Spawn watchdog that force-aborts after grace period.
                    let grace = self.grace_period;
                    tokio::spawn(async move {
                        tokio::time::sleep(grace).await;
                        let join_guard = handle.join.lock().await;
                        if let Some(jh) = join_guard.as_ref() {
                            jh.abort();
                        }
                    });
                    Ok(())
                } else {
                    // No cancel handle found — task may have just finished.
                    Err(TaskError::InvalidState)
                }
            }
        }
    }

    async fn subscribe_events(
        &self,
        id: TaskId,
        _from_seq: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError> {
        // Verify task exists.
        if !self.tasks.contains_key(&id) {
            return Err(TaskError::NotFound(id));
        }

        let stream = self.event_bus.subscribe(id);
        Ok(stream)
    }

    async fn list(&self, filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        self.store.list_snapshots(&filter)
    }
}
