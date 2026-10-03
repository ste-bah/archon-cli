//! The executor's half of a resume (#241): the agent runs with exactly the
//! confinement its spawn record names, or it does not run.
//!
//! The record arrives with the history in the resume slot. Two things are
//! decided from it rather than from the request:
//!
//! - the rung. A resume does not ask the automatic policy again: an agent
//!   that ran on the worktree rung because an overlap put it there runs there
//!   again after the overlap is gone, and a cap lowered since the spawn
//!   refuses the resume rather than clamping it;
//! - the whole effective confinement. Built the way the spawn built it, it is
//!   compared with the record field by field, and any difference refuses.

use archon_tools::isolation::{Isolation, IsolationTier};

use super::*;
use crate::agents::transcript::SpawnConfinement;

impl AgentSubagentExecutor {
    /// The spawn record of `manager_id` when this run resumes it.
    pub(super) async fn resume_pin(&self, manager_id: &str) -> Option<SpawnConfinement> {
        self.pending_resume_messages
            .lock()
            .await
            .get(manager_id)
            .map(|pending| pending.confinement.clone())
    }

    /// Mark the run failed and return `reason` as its error. A resume entry
    /// it leaves behind is dropped: no runner will take it, and a later spawn
    /// under the same id must not inherit its history.
    pub(super) async fn refuse_run(&self, manager_id: &str, reason: String) -> ExecutorError {
        self.pending_resume_messages.lock().await.remove(manager_id);
        let _ = self
            .subagent_manager
            .lock()
            .await
            .mark_failed(manager_id, reason.clone());
        ExecutorError::Internal(reason)
    }
}

/// The rung the run names explicitly: a resumed agent's recorded rung,
/// whatever its request asked; else the rung its request named, if any.
pub(super) fn explicit_tier(
    pin: Option<&SpawnConfinement>,
    asked: Option<Isolation>,
) -> Option<IsolationTier> {
    pin.map(|record| record.tier)
        .or_else(|| asked.and_then(Isolation::tier))
}

/// Refuse a resume that was granted a different rung from its recorded one.
/// Only `subagent.isolation_max_tier` can do that, by clamping it lower.
pub(super) fn check_rung(
    agent_id: &str,
    pin: Option<&SpawnConfinement>,
    granted: IsolationTier,
) -> Result<(), String> {
    match pin {
        Some(record) if record.tier != granted => Err(format!(
            "cannot resume agent '{agent_id}': it ran on the isolation rung '{recorded}', and \
             subagent.isolation_max_tier now grants only '{granted}'. A resume on a lower rung \
             would run it less isolated than it was spawned, so it is refused. To continue, \
             raise subagent.isolation_max_tier to at least '{recorded}', or spawn a new agent \
             and give it the transcript as context.",
            recorded = record.tier.as_str(),
            granted = granted.as_str(),
        )),
        _ => Ok(()),
    }
}

/// Refuse a resume whose effective confinement differs from its record.
pub(super) fn check_effective(
    agent_id: &str,
    pin: Option<&SpawnConfinement>,
    effective: &SpawnConfinement,
) -> Result<(), String> {
    let Some(record) = pin else {
        return Ok(());
    };
    let differing = record.differing_fields(effective);
    if differing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "cannot resume agent '{agent_id}': the confinement it would run with differs from \
         the one it was spawned with in {} (recorded {record:?}; now {effective:?}). A resume \
         runs with exactly the recorded confinement or not at all. To continue, spawn a new \
         agent with the recorded confinement and give it the transcript as context.",
        differing.join(", "),
    ))
}

#[cfg(test)]
#[path = "run_resume_tests.rs"]
mod tests;
