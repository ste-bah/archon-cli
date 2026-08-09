use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::board::{DelegatedOutcome, close_delegated_task};

/// How long [`drain_board_items`] waits for in-flight work before giving up.
///
/// Long enough for a subagent that has already finished to run its completion
/// tail — the case this exists for is a tail losing a race with `exit`, which is
/// microseconds of work, not seconds. Short enough that a genuinely stuck agent
/// cannot hold the process open: whatever is left is released rather than
/// waited on.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the drain re-checks. Small, because the expected wait is one tick.
const DRAIN_POLL: Duration = Duration::from_millis(50);

/// Close out mirrored board items before the process exits.
///
/// `handle_print_mode_if_requested` ends with `std::process::exit`, which runs
/// no destructors and does not let detached tokio tasks finish. A background
/// subagent dispatched through `TaskCreate` records its terminal status from
/// exactly such a task, and that is what closes its board item — so a subagent
/// finishing in the same instant the run ends leaves an item claimed by an agent
/// that no longer exists, forever. Observed live: `TASK-TDL-130`'s retry wrote
/// its file at 18:22, the run exited at 18:22, and the item was still `claimed`
/// twenty minutes later with the work sitting completed on disk.
///
/// Two phases, because the two cases want opposite things. **Wait first**: an
/// agent that has already finished only needs its tail scheduled, so a short
/// poll lets the normal path close the item with its true outcome. **Then
/// release**: anything still unfinished is genuinely unfinished, and returning
/// it to `open` with no holder says so — the same disposition the lease sweep
/// applies to a claim whose holder is gone, which is what this process is about
/// to become.
///
/// Releasing rather than resolving is deliberate. At exit this cannot know
/// whether the work succeeded, and marking it resolved would assert something
/// unverified; `open` is the honest answer and is the one that gets the item
/// picked up again.
pub async fn drain_board_items() {
    drain_board_items_within(DRAIN_TIMEOUT).await;
}

/// [`drain_board_items`] with an explicit budget, so a test can exercise the
/// give-up path without waiting the production timeout.
async fn drain_board_items_within(budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let pending = TASK_MANAGER.pending_board_items();
        if pending.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                count = pending.len(),
                "board items still claimed at exit; releasing them so they are not \
                 held by an agent this process is about to take with it"
            );
            for (task_id, item_id) in pending {
                tracing::debug!(%task_id, %item_id, "releasing board claim at exit");
                close_delegated_task(&item_id, DelegatedOutcome::Stopped);
            }
            return;
        }
        tokio::time::sleep(DRAIN_POLL).await;
    }
}

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
}

// ---------------------------------------------------------------------------
// TaskManager
// ---------------------------------------------------------------------------

pub struct TaskManager {
    tasks: Mutex<HashMap<String, TaskInfo>>,
    cancellation_tokens: Mutex<HashMap<String, Arc<AtomicBool>>>,
    execution_tokens: Mutex<HashMap<String, CancellationToken>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            cancellation_tokens: Mutex::new(HashMap::new()),
            execution_tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new task and return its 8-character ID.
    pub fn create_task(&self, description: &str) -> String {
        self.create_task_with_parent(description, None)
    }

    /// Create a task whose async work is cancelled with its optional parent.
    pub fn create_task_with_parent(
        &self,
        description: &str,
        parent: Option<&CancellationToken>,
    ) -> String {
        let full_uuid = Uuid::new_v4().to_string().replace('-', "");
        let id = full_uuid[..8].to_string();
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
    /// Task ids are eight characters; subagent ids are full dashed UUIDs minted
    /// by a different registry. An agent that spawns work and then asks about
    /// it holds the SUBAGENT id, and naturally shortens it — a live run called
    /// `TaskGet` with `8949f93a` while its subagent was
    /// `8949f93a-90bf-4d35-…`, and got "task not found" for work that existed.
    /// Exact lookup on one namespace cannot answer a question asked in the
    /// other.
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

        {
            let tokens = self
                .execution_tokens
                .lock()
                .map_err(|e| format!("lock poisoned: {e}"))?;
            let token = tokens
                .get(id)
                .ok_or_else(|| format!("task not found: {id}"))?;
            token.cancel();
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
        // What the board has to be told, decided under the lock and acted on
        // after it. A board write goes to storage, and holding the task map
        // across it would serialise every other task's status update behind a
        // database round-trip. Deciding here also means the mirror fires only
        // on a transition that was actually applied — an invalid transition
        // returns early and must not close an item out.
        let mut mirror = None;

        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(info) = tasks.get_mut(id)
        {
            if !is_valid_transition(&info.status, &status) {
                return;
            }
            info.status = status.clone();
            if status == TaskStatus::Completed
                || status == TaskStatus::Failed
                || status == TaskStatus::Stopped
            {
                info.completed_at = Some(Utc::now());
                if let Some(item) = info.board_item_id.clone() {
                    let outcome = match status {
                        TaskStatus::Completed => DelegatedOutcome::Completed,
                        TaskStatus::Stopped => DelegatedOutcome::Stopped,
                        _ => DelegatedOutcome::Failed,
                    };
                    mirror = Some((item, outcome));
                }
            }
        }

        if let Some((item, outcome)) = mirror {
            close_delegated_task(&item, outcome);
        }
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

#[cfg(test)]
mod resolve_tests {
    use super::*;

    /// The live failure: an agent held its SUBAGENT id, shortened it, and asked
    /// `TaskGet` about it. Task ids and subagent ids are separate namespaces, so
    /// exact lookup answered "task not found" for work that existed.
    #[test]
    fn resolves_by_agent_id_and_by_prefix_of_it() {
        let mgr = TaskManager::new();
        let task_id = mgr.create_task("decompose one spec");
        mgr.set_agent_id(&task_id, "8949f93a-90bf-4d35-aa9a-2a7156e43c16");

        assert!(mgr.resolve_task(&task_id).is_some(), "exact task id");
        assert!(
            mgr.resolve_task("8949f93a-90bf-4d35-aa9a-2a7156e43c16")
                .is_some(),
            "full subagent id"
        );
        assert!(
            mgr.resolve_task("8949f93a").is_some(),
            "shortened subagent id — the case that failed live"
        );
    }

    /// An ambiguous prefix must return nothing. Reporting the wrong task's
    /// status is worse than reporting none, and a caller can always pass more
    /// characters.
    #[test]
    fn an_ambiguous_prefix_resolves_to_nothing() {
        let mgr = TaskManager::new();
        let a = mgr.create_task("first");
        let b = mgr.create_task("second");
        mgr.set_agent_id(&a, "dead0000-0000-0000-0000-000000000001");
        mgr.set_agent_id(&b, "dead0000-0000-0000-0000-000000000002");

        assert!(
            mgr.resolve_task("dead0000").is_none(),
            "a prefix matching two tasks must not guess"
        );
        assert!(
            mgr.resolve_task("dead0000-0000-0000-0000-000000000002")
                .is_some(),
            "the full id still resolves"
        );
    }

    #[test]
    fn unknown_and_empty_ids_still_resolve_to_nothing() {
        let mgr = TaskManager::new();
        mgr.create_task("only task");
        assert!(mgr.resolve_task("nosuchid").is_none());
        assert!(mgr.resolve_task("   ").is_none());
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;

    /// The drain must not delay an exit that has nothing to wait for. Every run
    /// pays this cost, including the overwhelming majority that dispatched no
    /// background subagent at all.
    #[tokio::test]
    async fn nothing_pending_returns_without_waiting() {
        let started = Instant::now();
        drain_board_items_within(Duration::from_secs(5)).await;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "an empty drain must return immediately, not burn the budget"
        );
    }

    /// A task with no mirrored item is not something to wait for: the board has
    /// nothing recorded for it, so there is nothing to close.
    #[tokio::test]
    async fn a_task_without_a_board_item_is_not_pending() {
        let mgr = TaskManager::new();
        let id = mgr.create_task("no subagent, no mirror");
        mgr.set_status(&id, TaskStatus::Running);
        assert!(
            mgr.pending_board_items().is_empty(),
            "only mirrored tasks can strand a board item"
        );
    }

    /// The case this exists for: the completion tail lands during the drain, so
    /// the item closes by the normal path with its true outcome and the drain
    /// stops waiting.
    #[tokio::test]
    async fn a_task_that_finishes_during_the_drain_stops_being_pending() {
        let mgr = TaskManager::new();
        let id = mgr.create_task("dispatched in the background");
        mgr.set_board_item_id(&id, "agent-uuid-1");
        mgr.set_status(&id, TaskStatus::Running);
        assert_eq!(mgr.pending_board_items().len(), 1);

        // What the detached completion task does when it finally gets scheduled.
        mgr.set_status(&id, TaskStatus::Completed);
        assert!(
            mgr.pending_board_items().is_empty(),
            "a terminal task is no longer something the drain waits on"
        );
    }

    /// A stuck agent must not hold the process open. The budget expires, the
    /// straggler is released rather than waited on, and the drain returns.
    #[tokio::test]
    async fn a_task_that_never_finishes_gives_up_at_the_budget() {
        // Uses the global manager because `drain_board_items_within` reads it;
        // the task is left Running for the whole call on purpose.
        let id = TASK_MANAGER.create_task("an agent that never reports back");
        TASK_MANAGER.set_board_item_id(&id, "agent-uuid-stuck");
        TASK_MANAGER.set_status(&id, TaskStatus::Running);

        let started = Instant::now();
        drain_board_items_within(Duration::from_millis(200)).await;
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(200),
            "the drain must actually wait before giving up, or the tail never lands"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the drain must give up at its budget rather than block the exit"
        );

        // Leave the global clean for any other test in this binary.
        TASK_MANAGER.set_status(&id, TaskStatus::Stopped);
    }
}
