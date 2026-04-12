use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use uuid::Uuid;

/// Globally unique task identifier — newtype over UUID v4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Task lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
    Corrupted,
}

impl TaskState {
    /// Returns true if this is a terminal state (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished | Self::Failed | Self::Cancelled | Self::Corrupted
        )
    }
}

/// Opaque stream identifier for streaming result handles.
pub type StreamId = u64;

/// Reference to task result data — inline for small, file for large.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResultRef {
    /// Inline result string (size <= 64 KiB).
    pub inline: Option<String>,
    /// Path to large result file (.archon/tasks/{uuid}/result.bin).
    pub file_path: Option<PathBuf>,
    /// Streaming handle for live result consumption.
    pub streaming_handle: Option<StreamId>,
}

/// Resource usage sample (cpu, memory).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceSample {
    pub cpu_ms: u64,
    pub rss_bytes: u64,
}

/// Core task struct matching TECH-AGS-ASYNC data model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub agent_name: String,
    pub agent_version: Option<semver::Version>,
    pub input: serde_json::Value,
    pub state: TaskState,
    pub progress_pct: Option<f32>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_ref: Option<TaskResultRef>,
    pub error: Option<String>,
    pub owner: String,
    pub resource_usage: Option<ResourceSample>,
    pub cancel_token_ref: Option<String>,
}

/// Event kinds for task lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskEventKind {
    Started,
    Progress,
    Finished,
    Failed,
    Cancelled,
}

/// A single event in a task's lifecycle with monotonic seq.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskEvent {
    pub task_id: TaskId,
    /// Monotonic per-task sequence number (REQ-ASYNC-009).
    pub seq: u64,
    pub kind: TaskEventKind,
    pub payload: serde_json::Value,
    pub at: DateTime<Utc>,
}

/// Snapshot of task state for status queries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub agent_name: String,
    pub state: TaskState,
    pub progress_pct: Option<f32>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl From<&Task> for TaskSnapshot {
    fn from(t: &Task) -> Self {
        Self {
            id: t.id,
            agent_name: t.agent_name.clone(),
            state: t.state,
            progress_pct: t.progress_pct,
            created_at: t.created_at,
            started_at: t.started_at,
            finished_at: t.finished_at,
            error: t.error.clone(),
        }
    }
}

/// Filter criteria for task listing.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub state: Option<TaskState>,
    pub agent_name: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

/// Request to submit a new async task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitRequest {
    pub agent_name: String,
    pub agent_version: Option<semver::Version>,
    pub input: serde_json::Value,
    pub owner: String,
}

/// Task-system errors.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("invalid state transition")]
    InvalidState,
    #[error("task is still pending")]
    Pending,
    #[error("task already cancelled")]
    AlreadyCancelled,
    #[error("task data corrupted")]
    Corrupted,
    #[error("task queue full")]
    QueueFull,
    #[error("not yet implemented")]
    Unimplemented,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result stream for task output retrieval.
/// Full variants (File, Chunks) added in TASK-AGS-203.
#[derive(Debug)]
pub enum TaskResultStream {
    /// Small inline result.
    Inline(String),
}
