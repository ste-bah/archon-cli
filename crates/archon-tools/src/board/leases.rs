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

use super::TOP_LEVEL_AGENT;
use crate::background_agents::{PollOutcome, poll_subagent};

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
/// **One registry, one lookup, and that is the point of this function.** It used
/// to ask `TASK_MANAGER` and then `BACKGROUND_AGENTS`, because `TaskCreate` and
/// `AgentTool` happened to record their agents in different places — which meant
/// `archon-pipeline`, recording in neither, read as dead from birth and had its
/// claims released while it worked. A fan-out that has to grow an arm per spawn
/// path fails silently every time a new one is added, so the registration moved
/// to the one function every runner passes through
/// (`agent_tool::run::run_subagent_with_auto_background`) and the question moved
/// with it.
///
/// `TASK_MANAGER` still exists and is still right for what it is for — task
/// status, metadata, `/tasks` — but it is no longer asked about liveness.
#[must_use]
pub fn holder_liveness(agent_id: &str) -> HolderLiveness {
    // The top-level agent was never spawned as a subagent, so it is in no
    // registry. It is alive for as long as the process is, and "not found" must
    // not be read as dead here or the sweep would strip its own claims.
    if agent_id == TOP_LEVEL_AGENT {
        return HolderLiveness::Live;
    }

    // Asked by runtime subagent id, not by UUID: pipeline agents are keyed by
    // `{session}-{ordinal}-{agent}` and a UUID-only lookup cannot see them.
    match poll_subagent(agent_id) {
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
