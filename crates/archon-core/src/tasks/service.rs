use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use tokio_stream::Stream;

use crate::agents::registry::AgentRegistry;
use crate::tasks::models::{
    SubmitRequest, Task, TaskError, TaskEvent, TaskFilter, TaskId,
    TaskResultStream, TaskSnapshot, TaskState,
};
use crate::tasks::queue::TaskQueue;

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
}

impl DefaultTaskService {
    pub fn new(registry: Arc<AgentRegistry>, max_queue_size: usize) -> Self {
        Self {
            registry,
            tasks: Arc::new(DashMap::new()),
            seq_counters: Arc::new(DashMap::new()),
            max_queue_size,
            queue: None,
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
        }
    }

    /// Access the shared tasks map (used by downstream tasks for hot-cache).
    pub fn tasks(&self) -> &Arc<DashMap<TaskId, Task>> {
        &self.tasks
    }

    /// Access the shared seq counters (used by executor + events).
    pub fn seq_counters(&self) -> &Arc<DashMap<TaskId, AtomicU64>> {
        &self.seq_counters
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

        // 5. Initialize per-task seq counter (REQ-ASYNC-009).
        self.seq_counters.insert(task_id, AtomicU64::new(0));

        // 6. Enqueue into per-agent queue when configured.
        if let Some(ref queue) = self.queue {
            if let Err(e) = queue.enqueue(task_id, &task.agent_name) {
                // Roll back the in-memory inserts on queue failure.
                self.tasks.remove(&task_id);
                self.seq_counters.remove(&task_id);
                return Err(e);
            }
        }

        Ok(task_id)
    }

    async fn status(&self, _id: TaskId) -> Result<TaskSnapshot, TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn result(&self, _id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn subscribe_events(
        &self,
        _id: TaskId,
        _from_seq: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn list(&self, _filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        Err(TaskError::Unimplemented)
    }
}
