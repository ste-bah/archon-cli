use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::tasks::models::{TaskError, TaskId};

/// Configuration for per-agent queue behavior.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Maximum number of concurrently executing tasks per agent.
    pub max_concurrent: usize,
    /// Maximum number of pending (waiting) tasks per agent before QueueFull.
    pub queue_capacity: usize,
    /// Extra permits available when burst mode activates.
    pub burst_capacity: usize,
    /// How long a task must wait in the pending queue before burst permits
    /// become available.
    pub burst_threshold: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            queue_capacity: 1000,
            burst_capacity: 5,
            burst_threshold: Duration::from_secs(30),
        }
    }
}

/// Per-agent queue slot with semaphore for concurrency control.
struct AgentSlot {
    semaphore: Arc<Semaphore>,
    burst_semaphore: Arc<Semaphore>,
    pending: Mutex<VecDeque<PendingEntry>>,
    config: QueueConfig,
}

struct PendingEntry {
    task_id: TaskId,
    enqueued_at: Instant,
}

/// Trait for task queue operations.
pub trait TaskQueue: Send + Sync {
    /// Enqueue a task for the given agent. Returns `QueueFull` if at capacity.
    fn enqueue(&self, task_id: TaskId, agent_name: &str) -> Result<(), TaskError>;

    /// Try to dequeue a task for the given agent. Returns `None` if no tasks
    /// are pending or no permits are available.
    fn try_dequeue(&self, agent_name: &str) -> Option<(TaskId, OwnedSemaphorePermit)>;

    /// Remove a pending task (cancel-before-execution). Returns `true` if removed.
    fn remove_pending(&self, task_id: TaskId) -> bool;

    /// Number of pending (not yet running) tasks for an agent.
    fn depth(&self, agent_name: &str) -> usize;
}

/// Per-agent bounded queue with semaphore concurrency control and burst mode.
pub struct PerAgentTaskQueue {
    queues: DashMap<String, AgentSlot>,
    default_config: QueueConfig,
}

impl PerAgentTaskQueue {
    pub fn new(default_config: QueueConfig) -> Self {
        Self {
            queues: DashMap::new(),
            default_config,
        }
    }

    fn get_or_create_slot(
        &self,
        agent_name: &str,
    ) -> dashmap::mapref::one::Ref<'_, String, AgentSlot> {
        if !self.queues.contains_key(agent_name) {
            let config = self.default_config.clone();
            let slot = AgentSlot {
                semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
                burst_semaphore: Arc::new(Semaphore::new(config.burst_capacity)),
                pending: Mutex::new(VecDeque::new()),
                config,
            };
            self.queues.entry(agent_name.to_string()).or_insert(slot);
        }
        self.queues.get(agent_name).unwrap()
    }
}

impl TaskQueue for PerAgentTaskQueue {
    fn enqueue(&self, task_id: TaskId, agent_name: &str) -> Result<(), TaskError> {
        let slot = self.get_or_create_slot(agent_name);
        let mut pending = slot.pending.blocking_lock();
        if pending.len() >= slot.config.queue_capacity {
            return Err(TaskError::QueueFull);
        }
        pending.push_back(PendingEntry {
            task_id,
            enqueued_at: Instant::now(),
        });
        Ok(())
    }

    fn try_dequeue(&self, agent_name: &str) -> Option<(TaskId, OwnedSemaphorePermit)> {
        let slot = self.queues.get(agent_name)?;

        // Try normal permit first.
        let permit = match slot.semaphore.clone().try_acquire_owned().ok() {
            Some(p) => Some(p),
            None => {
                // Check burst mode: if oldest pending entry waited longer than
                // the burst threshold, try to acquire a burst permit.
                let pending = slot.pending.blocking_lock();
                if let Some(front) = pending.front() {
                    if front.enqueued_at.elapsed() >= slot.config.burst_threshold {
                        drop(pending);
                        slot.burst_semaphore.clone().try_acquire_owned().ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };

        let permit = permit?;
        let mut pending = slot.pending.blocking_lock();
        let entry = pending.pop_front()?;
        Some((entry.task_id, permit))
    }

    fn remove_pending(&self, task_id: TaskId) -> bool {
        for entry in self.queues.iter() {
            let mut pending = entry.value().pending.blocking_lock();
            if let Some(pos) = pending.iter().position(|e| e.task_id == task_id) {
                pending.remove(pos);
                return true;
            }
        }
        false
    }

    fn depth(&self, agent_name: &str) -> usize {
        self.queues
            .get(agent_name)
            .map(|slot| slot.pending.blocking_lock().len())
            .unwrap_or(0)
    }
}
