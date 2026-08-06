//! Claim leases: a claim is valid only while the agent holding it is alive.
//!
//! The alternative is a TTL, and a TTL on agent work is a number nobody can
//! pick — too short and a slow-but-healthy implementer loses an item it is
//! halfway through, too long and a dead agent parks a finding for the rest of
//! the run. The runtime already knows which agents are executing, so the lease
//! reads that instead of guessing.
//!
//! **Do not move this to the `SubagentStop` hook.** That hook fires from
//! `on_visible_complete`, and `agent_tool/run.rs` deliberately skips
//! `on_visible_complete` on the `SubagentOutcome::AutoBackgrounded` arm — an
//! agent that outlives the auto-background timer never fires it, and those are
//! precisely the long-running agents most likely to be holding a claim. A
//! sweep that reads liveness directly has no such hole.

use std::fmt;

use archon_memory::board::BoardAccess;
use archon_memory::types::MemoryError;
use uuid::Uuid;

use super::TOP_LEVEL_AGENT;
use crate::background_agents::{PollOutcome, poll_background_agent};
use crate::task_manager::TASK_MANAGER;

/// Whether an agent holding a claim is still executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderLiveness {
    Live,
    Dead,
}

impl fmt::Display for HolderLiveness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Live => "live",
            Self::Dead => "dead",
        })
    }
}

/// One claim the sweep took back, kept so the caller can say what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedClaim {
    pub item_id: String,
    pub title: String,
    /// The agent that was holding it when the sweep found it dead.
    pub holder: String,
}

/// Is the agent holding a claim still executing?
///
/// **Both registries are consulted, and that is the whole point of this
/// function.** Subagents spawned through `AgentTool` register a handle in
/// `BACKGROUND_AGENTS`; subagents spawned through `TaskCreate` do not — they go
/// straight to `run_subagent_foreground`, which never touches that registry, and
/// the only record of them is the `TASK_MANAGER` task that dispatched them.
/// Checking one registry alone reports every agent of the other kind as dead and
/// releases claims out from under agents that are still working.
#[must_use]
pub fn holder_liveness(agent_id: &str) -> HolderLiveness {
    // The top-level agent is in neither registry — those track subagents. It is
    // alive for as long as the process is, so "not found" must not be read as
    // dead here or the sweep would strip the top-level agent's own claims.
    if agent_id == TOP_LEVEL_AGENT {
        return HolderLiveness::Live;
    }

    // TaskCreate agents first: this is the registry that knows about them, and
    // it answers `None` rather than "dead" for ids it has never seen, so an
    // AgentTool agent falls through cleanly.
    if let Some(running) = TASK_MANAGER.agent_is_running(agent_id) {
        return if running {
            HolderLiveness::Live
        } else {
            HolderLiveness::Dead
        };
    }

    // `BACKGROUND_AGENTS` is keyed by UUID. An id that is not one was never in
    // it, and having already missed `TASK_MANAGER` it belongs to nothing this
    // process is running.
    let Ok(id) = Uuid::parse_str(agent_id) else {
        return HolderLiveness::Dead;
    };

    match poll_background_agent(&id) {
        PollOutcome::Running => HolderLiveness::Live,
        PollOutcome::Complete(_) => HolderLiveness::Dead,
        // `Unknown` is ambiguous and the ambiguity is not visible from here:
        // reaping is eager (`spawn_gc_task` removes terminal handles on a 60s
        // cadence and leaves no tombstone), so an id absent from the registry
        // is either an agent that finished and was swept up, or one that never
        // existed. Both resolve to the same verdict — nothing is executing
        // under that id now, so the claim must come back — but a future reader
        // should not mistake this arm for a lookup failure.
        PollOutcome::Unknown => HolderLiveness::Dead,
    }
}

/// Release every claim in `run_id` whose holder is no longer executing.
///
/// Returns what it took back. Items further along the lifecycle keep their
/// status — `release_board_claim` only reverts `claimed` to `open` — because
/// losing the agent is not the same as retracting the work it recorded.
pub fn release_dead_claims(
    board: &dyn BoardAccess,
    run_id: &str,
) -> Result<Vec<ReleasedClaim>, MemoryError> {
    // Every status, not just `claimed`: an item moved to `in_review` or
    // `gaps_remain` still has a holder, and that holder can still die.
    let items = board.list_board_items_by_run(run_id, &[])?;
    let mut released = Vec::new();

    for item in items {
        let Some(holder) = item.claimed_by.clone() else {
            continue;
        };
        if holder_liveness(&holder) == HolderLiveness::Live {
            continue;
        }
        let update = board.release_board_claim(&item.id)?;
        // `applied` is false when someone else released it between the list and
        // the write. Report only what this sweep actually did.
        if update.applied {
            tracing::info!(
                item_id = %item.id,
                holder = %holder,
                run_id = %run_id,
                "released a board claim held by an agent that is no longer running"
            );
            released.push(ReleasedClaim {
                item_id: item.id,
                title: item.title,
                holder,
            });
        }
    }

    Ok(released)
}
