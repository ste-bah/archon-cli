use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use archon_session::plan::PlanStore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use plan_persistence::{PlanTaskMetadata, TaskTransitionError};

#[path = "task_manager/checked_status.rs"]
mod checked_status;
mod drain;
#[path = "task_manager/plan_installation.rs"]
pub mod plan_installation;
#[path = "task_manager/plan_persistence.rs"]
mod plan_persistence;
#[cfg(any(test, feature = "test-support"))]
#[path = "task_manager/plan_restore.rs"]
mod plan_restore;
#[path = "task_manager/runtime.rs"]
mod runtime;

#[cfg(test)]
#[path = "task_manager/checked_status_tests.rs"]
mod checked_status_tests;
#[cfg(test)]
mod resolve_tests;

// `drain` is a private child, so re-export keeps `drain_board_items` on exactly
// the path its callers in `main_modes` already use: `task_manager::…`, not
// `task_manager::drain::…`. Nothing else in the child is public.
pub use drain::drain_board_items;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub output: String,
    pub cost: f64,
    /// The subagent this task dispatched, when it dispatched one.
    ///
    /// It is what links a user-facing task back to the agent doing the work —
    /// not a liveness signal. That comes from `BACKGROUND_AGENTS`, which every
    /// spawn path registers with, `TaskCreate` included.
    ///
    /// `serde(default)` so stored tasks written before the field existed still
    /// deserialise.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The board item mirroring this task, when it is mirrored.
    ///
    /// Set by the dispatch path once the item exists, and read back when the
    /// task reaches a terminal state so the item can be closed out. `None` is
    /// the normal case for a task that dispatched no subagent, and also for
    /// any process with no memory service open — mirroring is best-effort.
    #[serde(default)]
    pub board_item_id: Option<String>,
    /// Plan linkage is absent for manual, process-scoped tasks.
    #[serde(default)]
    pub metadata: Option<PlanTaskMetadata>,
}

// ---------------------------------------------------------------------------
// TaskManager
// ---------------------------------------------------------------------------

pub struct TaskManager {
    tasks: Mutex<HashMap<String, TaskInfo>>,
    cancellation_tokens: Mutex<HashMap<String, Arc<AtomicBool>>>,
    execution_tokens: Mutex<HashMap<String, CancellationToken>>,
    plan_persistence: Mutex<HashMap<String, PlanStore>>,
    plan_authorities: Mutex<HashMap<String, Arc<archon_session::plan::PlanApprovalAuthority>>>,
    #[cfg(any(test, feature = "test-support"))]
    fail_next_plan_installation: Mutex<Option<String>>,
}

/// Test-only cleanup guard for durable tasks installed into a shared manager.
///
/// It removes only the supplied task IDs and the supplied session's PlanStore
/// attachment when dropped, including while an assertion unwinds.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub struct ScopedPlanTaskCleanup<'a> {
    pub(super) manager: &'a TaskManager,
    pub(super) session_id: String,
    pub(super) task_ids: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for ScopedPlanTaskCleanup<'_> {
    fn drop(&mut self) {
        self.manager
            .cleanup_plan_tasks_for_test(&self.session_id, &self.task_ids);
    }
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            cancellation_tokens: Mutex::new(HashMap::new()),
            execution_tokens: Mutex::new(HashMap::new()),
            plan_persistence: Mutex::new(HashMap::new()),
            plan_authorities: Mutex::new(HashMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            fail_next_plan_installation: Mutex::new(None),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn attach_plan_store(
        &self,
        store: PlanStore,
        session_id: impl Into<String>,
    ) -> Result<(), TaskTransitionError> {
        self.attach_plan_store_for_test(store, session_id)
    }

    /// Create a new task and return its collision-resistant UUID ID.
    pub fn create_task(&self, description: &str) -> String {
        self.create_task_with_parent(description, None)
    }

    /// Create a task whose async work is cancelled with its optional parent.
    pub fn create_task_with_parent(
        &self,
        description: &str,
        parent: Option<&CancellationToken>,
    ) -> String {
        let id = Uuid::new_v4().simple().to_string();
        let execution_token = parent
            .map(CancellationToken::child_token)
            .unwrap_or_default();

        let info = TaskInfo {
            id: id.clone(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
            output: String::new(),
            cost: 0.0,
            agent_id: None,
            board_item_id: None,
            metadata: None,
        };

        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(id.clone(), info);
        }
        if let Ok(mut tokens) = self.cancellation_tokens.lock() {
            tokens.insert(id.clone(), Arc::new(AtomicBool::new(false)));
        }
        if let Ok(mut tokens) = self.execution_tokens.lock() {
            tokens.insert(id.clone(), execution_token);
        }

        id
    }

    /// Get a snapshot of a task's info.
    pub fn get_task(&self, id: &str) -> Option<TaskInfo> {
        self.tasks.lock().ok()?.get(id).cloned()
    }

    /// Resolve a task by task id, by the agent id it dispatched, or by an
    /// unambiguous prefix of either.
    ///
    /// Task IDs and subagent IDs are UUIDs minted by different registries. An
    /// agent that spawns work and then asks about it holds the subagent ID and
    /// may naturally shorten it. Exact lookup on one namespace cannot answer a
    /// question asked in the other.
    ///
    /// A prefix matching more than one task returns `None` rather than a guess:
    /// reporting the wrong task's status is worse than reporting none.
    pub fn resolve_task(&self, id: &str) -> Option<TaskInfo> {
        let needle = id.trim();
        if needle.is_empty() {
            return None;
        }
        let tasks = self.tasks.lock().ok()?;
        if let Some(info) = tasks.get(needle) {
            return Some(info.clone());
        }
        if let Some(info) = tasks
            .values()
            .find(|info| info.agent_id.as_deref() == Some(needle))
        {
            return Some(info.clone());
        }
        // Prefix, over both namespaces, and only when it identifies one task.
        let mut matches = tasks.values().filter(|info| {
            info.id.starts_with(needle)
                || info
                    .agent_id
                    .as_deref()
                    .is_some_and(|agent| agent.starts_with(needle))
        });
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first.clone())
    }

    /// Record the subagent a task dispatched, so its liveness stays findable.
    ///
    /// The task id and the subagent id are two unrelated UUIDs minted moments
    /// apart in `TaskCreate`; nothing else ever writes the association down,
    /// and once `execute` returns the link is gone.
    pub fn set_agent_id(&self, id: &str, agent_id: &str) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id)
        {
            info.agent_id = Some(agent_id.to_string());
        }
    }

    /// Is the *task* that dispatched `agent_id` still open?
    ///
    /// **Not the answer to "is that agent executing".** That is
    /// `board::leases::holder_liveness`, which reads `BACKGROUND_AGENTS` and
    /// nothing else; deriving liveness from here as well is the fan-out issue
    /// #129 removed. This reports what the task board of `/tasks` shows, which
    /// can lag the runner in either direction.
    ///
    /// `None` means no task in this process ever dispatched that id.
    ///
    /// `Pending` counts as open: the task has been dispatched and the runner
    /// simply has not reported back yet.
    pub fn agent_is_running(&self, agent_id: &str) -> Option<bool> {
        let tasks = self.tasks.lock().ok()?;
        tasks
            .values()
            .find(|info| info.agent_id.as_deref() == Some(agent_id))
            .map(|info| matches!(info.status, TaskStatus::Pending | TaskStatus::Running))
    }

    /// Update a task's description.
    pub fn update_task(
        &self,
        id: &str,
        description: Option<&str>,
    ) -> Result<(), TaskTransitionError> {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        let info = tasks
            .get_mut(id)
            .ok_or_else(|| TaskTransitionError::NotFound(id.to_string()))?;

        if description.is_some() && info.metadata.is_some() {
            return Err(TaskTransitionError::PlanTaskDescriptionImmutable(
                id.to_string(),
            ));
        }
        if let Some(description) = description {
            info.description = description.to_string();
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
        runtime::stop_task(self, id)
    }

    /// Get captured output, optionally with offset (byte-based).
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

    /// Legacy adapter for tool payloads that already separate durable evidence
    /// IDs from their run. New callers should use [`Self::set_status_checked`].
    pub fn set_status_checked_with_evidence_ids(
        &self,
        id: &str,
        status: TaskStatus,
        evidence_run_id: &str,
        evidence_ids: &[String],
    ) -> Result<(), TaskTransitionError> {
        let task = self
            .get_task(id)
            .ok_or_else(|| TaskTransitionError::NotFound(id.to_string()))?;
        let trusted = match &task.metadata {
            Some(metadata) if status == TaskStatus::Completed => {
                let (store, authority) = {
                    let persistence = self
                        .plan_persistence
                        .lock()
                        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
                    let store =
                        persistence
                            .get(&metadata.session_id)
                            .cloned()
                            .ok_or_else(|| {
                                TaskTransitionError::Persistence(format!(
                                    "plan-linked task session {} has no attached plan store",
                                    metadata.session_id
                                ))
                            })?;
                    drop(persistence);
                    let authorities = self
                        .plan_authorities
                        .lock()
                        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
                    let authority = authorities.get(&metadata.session_id).cloned().ok_or_else(
                        || {
                            TaskTransitionError::Persistence(format!(
                                "plan-linked task session {} has no attached approval authority",
                                metadata.session_id
                            ))
                        },
                    )?;
                    (store, authority)
                };
                store
                    .resolve_required_evidence(
                        &authority,
                        &metadata.session_id,
                        evidence_run_id,
                        evidence_ids,
                        &metadata.required_evidence,
                    )
                    .map_err(|error| {
                        if error.kind() == std::io::ErrorKind::PermissionDenied {
                            TaskTransitionError::UntrustedEvidence(error.to_string())
                        } else {
                            TaskTransitionError::EvidenceResolution(error.to_string())
                        }
                    })?
            }
            _ => Vec::new(),
        };
        plan_persistence::set_status_checked(self, id, status, &trusted)
    }

    /// Set the status of a task and report invalid or persistence failures.
    pub fn set_status(&self, id: &str, status: TaskStatus) -> Result<(), TaskTransitionError> {
        if self
            .get_task(id)
            .is_some_and(|task| task.metadata.is_some())
        {
            return self.set_status_checked_with_evidence_ids(id, status, "", &[]);
        }
        runtime::set_status(self, id, status)
    }

    /// Mirrored board items whose task has not reached a terminal status.
    ///
    /// Returned as `(task_id, board_item_id)` so a caller can report which task
    /// it is still waiting on rather than only how many.
    pub fn pending_board_items(&self) -> Vec<(String, String)> {
        let Ok(tasks) = self.tasks.lock() else {
            return Vec::new();
        };
        tasks
            .values()
            .filter(|info| matches!(info.status, TaskStatus::Pending | TaskStatus::Running))
            .filter_map(|info| {
                info.board_item_id
                    .as_ref()
                    .map(|item| (info.id.clone(), item.clone()))
            })
            .collect()
    }

    /// Record the board item mirroring this task.
    ///
    /// Separate from [`Self::set_agent_id`] because the two can fail
    /// independently: a task always has a subagent id once it dispatches one,
    /// but it only has a board item if the board was reachable.
    pub fn set_board_item_id(&self, id: &str, board_item_id: &str) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id)
        {
            info.board_item_id = Some(board_item_id.to_string());
        }
    }

    /// Append text to a task's output buffer, capped at 1 MB.
    pub fn append_output(&self, id: &str, text: &str) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id)
        {
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

    /// Get the cancellation token for a task (for passing to polling workers).
    pub fn cancellation_token(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.cancellation_tokens.lock().ok()?.get(id).cloned()
    }

    /// Get the task-owned token used to cancel async execution.
    pub fn execution_token(&self, id: &str) -> Option<CancellationToken> {
        self.execution_tokens.lock().ok()?.get(id).cloned()
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}
