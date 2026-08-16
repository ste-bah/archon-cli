//! Board tools — the surface a subagent uses to reach the task board.
//!
//! The storage layer (`archon_memory::board`) has had a compare-and-set claim
//! and a run-partitioned relation since it landed, and nothing outside
//! `archon-memory` called it. This module is that caller: four tools that let
//! an agent raise a finding, take ownership of one, read the board, and close
//! an item out.
//!
//! Two things here are load-bearing and neither is obvious from the tool
//! schemas.
//!
//! **Attribution comes from [`ToolContext::subagent_id`], never from the
//! model.** A tool argument naming the writer would be a field the caller can
//! fill in with anything, and a board whose `raised_by` is self-reported cannot
//! support a lease — liveness is only checkable against an id the runtime
//! minted. `None` is the top-level agent, which is a real answer and is
//! recorded as [`TOP_LEVEL_AGENT`] rather than refused.
//!
//! **A claim is a lease against a live process**, not a durable assertion. See
//! [`leases`] for how a holder's liveness is established and why one registry
//! is not enough.

use std::sync::{Arc, OnceLock};

use archon_memory::board::BoardAccess;

/// `pub(crate)` so write-intent claims can ask the same liveness question the
/// board's own lease sweep asks, from the one registry (#184 M2). Exposing the
/// module rather than re-deriving liveness is the point — `leases.rs` documents
/// at length why a second opinion here goes wrong.
pub(crate) mod leases;
mod mirror;
mod tools;
mod tools_lifecycle;

#[cfg(test)]
#[path = "board/board_tool_tests.rs"]
mod board_tool_tests;
#[cfg(test)]
#[path = "board/leases_tests.rs"]
mod leases_tests;

pub use leases::{HolderLiveness, ReleasedClaim, holder_liveness, release_dead_claims};
pub use mirror::{
    DelegatedOutcome, close_delegated_task, raise_delegated_branch, raise_delegated_task,
};
pub use tools::{BoardListTool, BoardRaiseTool};
pub use tools_lifecycle::{BoardClaimTool, BoardResolveTool};

/// What a write is attributed to when the caller is the top-level agent.
///
/// Spelled out rather than left null so a board row always names somebody. The
/// lease sweep also special-cases this value: the top-level agent is not in any
/// subagent registry, and treating "absent from the registry" as dead would
/// have it release its own claims on the next sweep.
pub const TOP_LEVEL_AGENT: &str = "top-level-agent";

/// The board handle installed by the binary layer.
///
/// A process-global rather than a `create_default_registry` parameter, matching
/// how the subagent executor and the game-theory executor are installed: the
/// registry is built in ~25 places, most of them tests that have no memory
/// service, and threading a handle through all of them would make every one of
/// them care about a board they never touch.
static BOARD_ACCESS: OnceLock<Arc<dyn BoardAccess>> = OnceLock::new();

/// Install the process-wide board handle. Later calls are ignored.
///
/// Called once at session boot, from whichever side of `MemoryAccess` this
/// process ended up on — the claim CAS is only global because the remote arm
/// resolves in the one process that owns the CozoDB writer.
pub fn install_board_access(access: Arc<dyn BoardAccess>) {
    if BOARD_ACCESS.set(access).is_err() {
        tracing::debug!("board access already installed; keeping the first handle");
    }
}

/// How a board tool reaches storage.
///
/// The global arm resolves at call time rather than at construction, because
/// `create_default_registry` runs before memory is opened. The direct arm
/// exists so tests can hand a tool its own in-memory graph — a `OnceLock` can
/// only be set once per process, and a test that had to share the global with
/// every other test in the binary would be ordering-dependent.
#[derive(Clone)]
pub enum BoardHandle {
    Global,
    Direct(Arc<dyn BoardAccess>),
}

impl BoardHandle {
    pub fn resolve(&self) -> Result<Arc<dyn BoardAccess>, String> {
        match self {
            Self::Direct(access) => Ok(Arc::clone(access)),
            Self::Global => BOARD_ACCESS.get().cloned().ok_or_else(|| {
                "the task board is unavailable: no memory service is open in this process"
                    .to_string()
            }),
        }
    }
}

/// The run that owns a session's slice of the board.
///
/// Every board item is partitioned by `run_id`, and a subagent has no way to
/// learn its run other than from the session it inherited — `session_id` is
/// copied verbatim down the whole tree, which is exactly what makes it usable
/// here.
///
/// A workflow stage session is `{run_id}-stage-{stage}-attempt-{n}`
/// (`src/command/workflow_live_runner.rs`, `workflow_agent_session_id`), so the
/// run is everything before the first `-stage-`. Any other session id — a plain
/// interactive one — has no such prefix and *is* the run: one interactive
/// session is one run, and its subagents inherit it unchanged.
///
/// The first occurrence is deliberate. A stage id may itself contain
/// `-stage-` (it is only sanitised to alphanumerics, `-` and `_`), so
/// splitting on the last occurrence would fold part of the stage name into the
/// run and split one run's board across several partitions.
#[must_use]
pub fn run_id_for_session(session_id: &str) -> &str {
    match session_id.find("-stage-") {
        // A leading `-stage-` would leave an empty run id, which the storage
        // layer refuses outright. Fall back to the whole session so a
        // malformed id degrades to "its own run" instead of to an error the
        // agent cannot act on.
        Some(0) | None => session_id,
        Some(index) => &session_id[..index],
    }
}

/// The id a board write is attributed to.
///
/// Reads the runtime's own id for the caller. `None` means the top-level agent
/// made the call directly — legitimate, and recorded as such.
#[must_use]
pub fn caller_id(ctx: &crate::tool::ToolContext) -> String {
    ctx.subagent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| TOP_LEVEL_AGENT.to_string())
}
