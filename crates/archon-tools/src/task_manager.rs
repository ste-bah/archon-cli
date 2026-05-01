use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Global TaskManager instance
// ---------------------------------------------------------------------------

/// Global task manager accessible from all tool implementations.
pub static TASK_MANAGER: LazyLock<TaskManager> = LazyLock::new(TaskManager::new);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Maximum bytes stored in a task's output buffer.
const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Stopped,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "Pending"),
            TaskStatus::Running => write!(f, "Running"),
            TaskStatus::Completed => write!(f, "Completed"),
            TaskStatus::Failed => write!(f, "Failed"),
            TaskStatus::Stopped => write!(f, "Stopped"),
        }
    }
}

/// Returns true if transitioning from `from` to `to` is valid.
///
/// Valid transitions:
///   Pending  -> Running | Failed | Stopped
///   Running  -> Completed | Failed | Stopped
///
/// Terminal states (Completed, Failed, Stopped) cannot transition further.
fn is_valid_transition(from: &TaskStatus, to: &TaskStatus) -> bool {
    matches!(
        (from, to),
        (TaskStatus::Pending, TaskStatus::Running)
            | (TaskStatus::Pending, TaskStatus::Failed)
            | (TaskStatus::Pending, TaskStatus::Stopped)
            | (TaskStatus::Running, TaskStatus::Completed)
            | (TaskStatus::Running, TaskStatus::Failed)
            | (TaskStatus::Running, TaskStatus::Stopped)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output: String,
    pub cost: f64,
}

// ---------------------------------------------------------------------------
// TaskManager
// ---------------------------------------------------------------------------

pub struct TaskManager {
    tasks: Mutex<HashMap<String, TaskInfo>>,
    cancellation_tokens: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            cancellation_tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new task and return its 8-character ID.
    pub fn create_task(&self, description: &str) -> String {
        let full_uuid = Uuid::new_v4().to_string().replace('-', "");
        let id = full_uuid[..8].to_string();

        let info = TaskInfo {
            id: id.clone(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
            output: String::new(),
            cost: 0.0,
        };

        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(id.clone(), info);
        }
        if let Ok(mut tokens) = self.cancellation_tokens.lock() {
            tokens.insert(id.clone(), Arc::new(AtomicBool::new(false)));
        }

        id
    }

    /// Get a snapshot of a task's info.
    pub fn get_task(&self, id: &str) -> Option<TaskInfo> {
        self.tasks.lock().ok()?.get(id).cloned()
    }

    /// Update a task's description.
    pub fn update_task(&self, id: &str, description: Option<&str>) -> Result<(), String> {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?;

        let info = tasks
            .get_mut(id)
            .ok_or_else(|| format!("task not found: {id}"))?;

        if let Some(desc) = description {
            info.description = desc.to_string();
        }

        Ok(())
    }

    /// List all tasks.
    pub fn list_tasks(&self) -> Vec<TaskInfo> {
        match self.tasks.lock() {
            Ok(tasks) => tasks.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Stop a task by setting its cancellation token and status.
    pub fn stop_task(&self, id: &str) -> Result<(), String> {
        // Set cancellation token
        {
            let tokens = self
                .cancellation_tokens
                .lock()
                .map_err(|e| format!("lock poisoned: {e}"))?;

            let token = tokens
                .get(id)
                .ok_or_else(|| format!("task not found: {id}"))?;

            token.store(true, Ordering::SeqCst);
        }

        // Update status
        self.set_status(id, TaskStatus::Stopped);

        Ok(())
    }

    /// Get captured output, optionally with offset and limit (byte-based).
    pub fn get_output(
        &self,
        id: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, String> {
        let tasks = self
            .tasks
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?;

        let info = tasks
            .get(id)
            .ok_or_else(|| format!("task not found: {id}"))?;

        let output = &info.output;
        let start = offset.unwrap_or(0).min(output.len());
        let end = match limit {
            Some(lim) => (start + lim).min(output.len()),
            None => output.len(),
        };

        Ok(output[start..end].to_string())
    }

    /// Set the status of a task. Invalid transitions are silently ignored.
    pub fn set_status(&self, id: &str, status: TaskStatus) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id) {
                if !is_valid_transition(&info.status, &status) {
                    return;
                }
                info.status = status.clone();
                if status == TaskStatus::Completed
                    || status == TaskStatus::Failed
                    || status == TaskStatus::Stopped
                {
                    info.completed_at = Some(Utc::now());
                }
            }
    }

    /// Append text to a task's output buffer, capped at 1 MB.
    pub fn append_output(&self, id: &str, text: &str) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id) {
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(info.output.len());
                if remaining > 0 {
                    let to_append = if text.len() > remaining {
                        &text[..remaining]
                    } else {
                        text
                    };
                    info.output.push_str(to_append);
                }
            }
    }

    /// Check if a task's cancellation token has been set.
    pub fn is_cancelled(&self, id: &str) -> bool {
        match self.cancellation_tokens.lock() {
            Ok(tokens) => tokens
                .get(id)
                .map(|t| t.load(Ordering::SeqCst))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Get the cancellation token for a task (for passing to async workers).
    pub fn cancellation_token(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.cancellation_tokens.lock().ok()?.get(id).cloned()
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}
