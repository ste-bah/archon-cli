//! Spawn-time coordination for the `Agent` tool (#184 M2).
//!
//! Two things an agent may declare when it is spawned: what it intends to
//! write, and which existing task it is taking on. Split out of `core.rs` to
//! keep that file under the 500-line gate.

use tracing::warn;

use crate::subagent_executor::SubagentOutcome;
use crate::task_manager::{TASK_MANAGER, TaskStatus};

/// Record this agent's declared writes and describe any live overlap.
///
/// Called before anything is spawned: coordination at dispatch time beats
/// reconciliation at merge time, and once both agents are running the cheap
/// moment has passed.
///
/// Advisory by design — the spawn is never refused. Making a declaration cost
/// you the spawn would teach models to stop declaring, and a silent agent is
/// worse than one that overlaps loudly.
pub(super) fn declare_writes(
    subagent_id: &str,
    label: Option<&str>,
    intended_writes: &[String],
) -> Option<String> {
    if intended_writes.is_empty() {
        return None;
    }
    let overlaps = crate::write_claims::claim(subagent_id, label, intended_writes);
    (!overlaps.is_empty()).then(|| crate::write_claims::describe_overlaps(&overlaps))
}

/// Record that `agent_id` has taken on `task_id`.
///
/// Best-effort, and deliberately quiet about two things:
///
/// - An unknown id is not an error. `resolve_task` accepts a task id, an agent
///   id or an unambiguous prefix, and a model naming something stale should not
///   lose its spawn over it.
/// - `set_status` ignores an invalid transition — terminal states are absorbing
///   and `Running -> Running` is not legal. Claiming an already-running task
///   therefore records the new owner without disturbing its status, which is
///   the honest outcome when two agents claim one task.
pub(super) fn claim_task_for_agent(task_id: &str, agent_id: &str) {
    let Some(resolved) = TASK_MANAGER.resolve_task(task_id) else {
        warn!(
            task_id,
            "Agent referenced a task that could not be resolved"
        );
        return;
    };
    TASK_MANAGER.set_agent_id(&resolved.id, agent_id);
    let _ = TASK_MANAGER.set_status(&resolved.id, TaskStatus::Running);
}

/// Settle a claimed task on the agent's terminal outcome.
///
/// `AutoBackgrounded` is deliberately left `Running`: the agent is still
/// working, and `TaskStatus`'s terminal states are absorbing, so marking it
/// finished would be a lie nothing could later undo.
pub(super) fn settle_claimed_task(task_id: Option<&String>, outcome: &SubagentOutcome) {
    let Some(task_id) = task_id else { return };
    match outcome {
        SubagentOutcome::Completed(_) => {
            let _ = TASK_MANAGER.set_status(task_id, TaskStatus::Completed);
        }
        SubagentOutcome::Failed(_) | SubagentOutcome::Cancelled => {
            let _ = TASK_MANAGER.set_status(task_id, TaskStatus::Failed);
        }
        SubagentOutcome::AutoBackgrounded => {}
    }
}
