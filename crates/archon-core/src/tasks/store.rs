use dashmap::DashMap;

use crate::tasks::models::{Task, TaskError, TaskFilter, TaskId, TaskSnapshot};

/// Trait for task state storage and retrieval.
///
/// The hot-cache (DashMap) serves reads; persistence is added in TASK-AGS-203.
pub trait TaskStateStore: Send + Sync {
    /// Get a snapshot of a single task by ID.
    fn get_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError>;

    /// List task snapshots matching the given filter.
    fn list_snapshots(&self, filter: &TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError>;

    /// Insert or update a task snapshot in the store.
    fn put_snapshot(&self, task: &Task) -> Result<(), TaskError>;
}

/// In-memory implementation backed by DashMap (hot-cache).
///
/// Used directly by DefaultTaskService. TASK-AGS-203 adds SqliteTaskStateStore
/// for durable persistence with this same trait.
pub struct InMemoryTaskStateStore {
    snapshots: DashMap<TaskId, TaskSnapshot>,
}

impl InMemoryTaskStateStore {
    pub fn new() -> Self {
        Self {
            snapshots: DashMap::new(),
        }
    }
}

impl Default for InMemoryTaskStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStateStore for InMemoryTaskStateStore {
    fn get_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        self.snapshots
            .get(&id)
            .map(|r| r.value().clone())
            .ok_or(TaskError::NotFound(id))
    }

    fn list_snapshots(&self, filter: &TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        let mut results: Vec<TaskSnapshot> = self
            .snapshots
            .iter()
            .map(|r| r.value().clone())
            .filter(|snap| {
                if let Some(state) = &filter.state {
                    if snap.state != *state {
                        return false;
                    }
                }
                if let Some(agent) = &filter.agent_name {
                    if snap.agent_name != *agent {
                        return false;
                    }
                }
                if let Some(since) = &filter.since {
                    if snap.created_at < *since {
                        return false;
                    }
                }
                true
            })
            .collect();
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    fn put_snapshot(&self, task: &Task) -> Result<(), TaskError> {
        let snapshot = TaskSnapshot::from(task);
        self.snapshots.insert(task.id, snapshot);
        Ok(())
    }
}
