//! TASK-AGS-105: `SubagentExecutor` trait — the archon-tools seam that
//! `AgentTool::execute` and `TaskCreateTool::execute` call into when
//! they need to actually run a subagent to completion.
//!
//! archon-tools does NOT depend on archon-core; instead, archon-core
//! installs a concrete `AgentSubagentExecutor` into the process-global
//! OnceLock at startup (from `Agent::new`). Tool code resolves it at
//! call time via `get_subagent_executor`. Tests install a
//! `NoopSubagentExecutor` via `install_subagent_executor` directly.
//!
//! The trait is split into TWO terminal side-effect methods per
//! Sherlock G3a Blocker-1 resolution (see mapping doc Section 2a):
//!
//! - `on_inner_complete` fires UNCONDITIONALLY from the tail of
//!   `run_to_completion`. Owns SubagentManager update and
//!   `save_agent_memory`. Preserves PRESERVE-D8 (save_agent_memory on
//!   every completion, including post-timer-abandonment).
//! - `on_visible_complete` fires ONLY from `run_subagent`'s non-timer
//!   completion arms (Completed / Failed / Cancelled). Owns hook fires
//!   (TeammateIdle, SubagentStop, TaskCompleted) and worktree cleanup.
//!   NOT called on the `AutoBackgrounded` select! arm — preserves
//!   PRESERVE-D5 (abandoned auto-backgrounded agents get inner side
//!   effects but NOT visible hooks / worktree cleanup).

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::agent_tool::SubagentRequest;
use crate::tool::ToolContext;

// ---------------------------------------------------------------------------
// Error + outcome side effect types.
// ---------------------------------------------------------------------------

/// Error surface returned by `SubagentExecutor::run_to_completion`.
#[derive(Debug, Clone)]
pub enum ExecutorError {
    /// No executor is installed in the process. This happens when
    /// `AgentTool::execute` or `TaskCreateTool::execute` is invoked
    /// outside of an `Agent::new`-initialized process (for example,
    /// from a test binary that forgot to call
    /// `install_subagent_executor`).
    NotInstalled,
    /// An internal executor error. The string is the user-facing
    /// reason surfaced in the resulting `ToolResult::error`.
    Internal(String),
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "subagent executor not installed"),
            Self::Internal(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Classification returned by `SubagentExecutor::classify`. Determines
/// whether `AgentTool::execute` returns a spawn marker synchronously
/// (`ExplicitBackground`) or awaits the subagent's completion
/// (`Foreground`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentClassification {
    /// Foreground path: caller awaits the completion outcome.
    Foreground,
    /// Explicit background path: caller returns a spawn marker
    /// immediately. Side effects still fire on the spawned task when
    /// the runner eventually completes.
    ExplicitBackground,
}

/// High-level outcome returned by `run_subagent` to its caller in
/// `AgentTool::execute` / `TaskCreateTool::execute`. Encodes the four
/// terminal arms of the AGT-025 `tokio::select!` race.
#[derive(Debug, Clone)]
pub enum SubagentOutcome {
    /// Runner finished successfully before the timer (or with no
    /// timer). Carries the final text to return to the caller.
    Completed(String),
    /// Runner finished with an error before the timer (or with no
    /// timer). Carries the user-facing reason.
    Failed(String),
    /// Timer fired before the runner finished; the join handle was
    /// abandoned. The runner continues executing in its own task;
    /// `on_inner_complete` fires from its tail when it eventually
    /// completes, but `on_visible_complete` does NOT.
    AutoBackgrounded,
    /// The cancel token was tripped before the runner finished.
    Cancelled,
}

/// Side effects returned by `on_visible_complete`. The executor has
/// already fired hooks + cleaned up worktrees; the struct only carries
/// the optional text suffix that the caller appends to the `Completed`
/// `ToolResult::success` content (for worktree-preserved-with-changes
/// notes — see mapping doc Section 1d W2/W5).
#[derive(Debug, Default, Clone)]
pub struct OutcomeSideEffects {
    /// Optional suffix to append to the Completed text (e.g. the
    /// `[Worktree: ... (branch: ...)]` note when the worktree had
    /// uncommitted changes and could not be auto-removed).
    pub text_suffix: Option<String>,
}

// ---------------------------------------------------------------------------
// The trait.
// ---------------------------------------------------------------------------

/// The `SubagentExecutor` trait: archon-tools calls into a concrete
/// implementor (installed by archon-core at process start) to actually
/// run a subagent. Exactly five methods — `run_to_completion`,
/// `on_inner_complete`, `on_visible_complete`, `auto_background_ms`,
/// `classify`.
#[async_trait]
pub trait SubagentExecutor: Send + Sync {
    /// Execute the subagent to completion, respecting the cancel
    /// token. This method does NOT fire visible hooks (TeammateIdle,
    /// SubagentStop, TaskCompleted) or clean up worktrees — those are
    /// `on_visible_complete`'s job.
    ///
    /// This method DOES call `on_inner_complete` at its tail,
    /// unconditionally, before returning. SubagentManager update +
    /// `save_agent_memory` fire there. The unconditional tail call is
    /// what preserves PRESERVE-D8 on the post-timer-abandonment path:
    /// even if `run_subagent` returned `AutoBackgrounded` to its
    /// caller, the runner continues executing and its `on_inner_complete`
    /// fire from this method's tail still runs.
    ///
    /// `SubagentStart` + `TaskCreated` (if `ctx.nested`) fire at the
    /// TOP of this method, BEFORE the early-return checks for
    /// fork-in-fork / cwd-worktree / MCP pre-flight. The top fires are
    /// NOT part of `on_inner_complete` / `on_visible_complete`.
    ///
    /// `subagent_id` is pre-allocated by `AgentTool::execute` and
    /// threaded in from `run_subagent` so the AutoBackgrounded marker
    /// returned to the caller can reference the exact id (no late
    /// binding dance).
    async fn run_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError>;

    /// Inner terminal side effects: `SubagentManager` update +
    /// `save_agent_memory`.
    ///
    /// Called from the TAIL of `run_to_completion` UNCONDITIONALLY.
    /// Collapse map: M1 (explicit-bg) + M2 (auto-bg spawn) + M3
    /// (foreground) → this single call.
    async fn on_inner_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
    );

    /// Visible terminal side effects: hooks (TeammateIdle,
    /// SubagentStop, TaskCompleted) + worktree cleanup.
    ///
    /// Called from `run_subagent`'s `select!` Completed/Failed/Cancelled
    /// arms ONLY. NOT called on the `AutoBackgrounded` timer arm —
    /// preserves PRESERVE-D5 post-abandonment semantics.
    ///
    /// `nested` controls `TaskCompleted` gating (fires only if
    /// `nested == true`, matching the old H5/H9 hook sites).
    async fn on_visible_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
        nested: bool,
    ) -> OutcomeSideEffects;

    /// Auto-background timeout in milliseconds. Returns 0 when the
    /// `ARCHON_AUTO_BACKGROUND_TASKS` env gate is disabled (so
    /// `run_subagent` takes the no-timer branch of the `select!`).
    fn auto_background_ms(&self) -> u64;

    /// Classify a request as foreground vs. explicit-background.
    /// Called by `AgentTool::execute` BEFORE spawning `run_subagent`
    /// so the tool can fork between the immediate-return background
    /// path and the await-outcome foreground path.
    fn classify(&self, request: &SubagentRequest) -> SubagentClassification;
}

// ---------------------------------------------------------------------------
// Process-global executor registry.
// ---------------------------------------------------------------------------

// TODO(post-105): switch to parking_lot::RwLock<Option<Arc<dyn SubagentExecutor>>>
// when a future test needs swappable executor state (e.g. RecordingExecutor
// asserting "was save_agent_memory called N times with tags X?"). OnceLock
// only permits install-once-per-process — swappable state requires a
// RwLock + serial_test::serial guard. Do NOT refactor speculatively.
static SUBAGENT_EXECUTOR: OnceLock<Arc<dyn SubagentExecutor>> = OnceLock::new();

/// Install a subagent executor for the process. First installer wins;
/// subsequent calls are no-ops (documented install-once-per-process
/// semantics).
pub fn install_subagent_executor(exec: Arc<dyn SubagentExecutor>) {
    // get_or_init expects `Fn() -> T`, so we wrap the clone in a closure.
    // We deliberately ignore the return value — callers don't need to
    // know whether their install was the winning one.
    let _ = SUBAGENT_EXECUTOR.get_or_init(|| exec);
}

/// Resolve the process-global subagent executor, if installed.
pub fn get_subagent_executor() -> Option<Arc<dyn SubagentExecutor>> {
    SUBAGENT_EXECUTOR.get().cloned()
}
