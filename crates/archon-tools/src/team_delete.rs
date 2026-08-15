//! `TeamDelete` — shut the team down, then remove it (TASK-CLI-312, #184 M5).
//!
//! It used to be one `remove_dir_all`. With nothing seated on the roster that
//! was harmless; now that agents are, deleting first would strand every one of
//! them — still running, still writing, on a roster nothing reads.
//!
//! So it is a handshake, the shape Claude Code uses: `shutdown_request` to each
//! member, members leave the roster as they reach a terminal state, and the
//! directory goes last. A member that does not stop within the grace period
//! leaves the team intact and is named in the refusal — a half-deleted team is
//! worse than one that is still there.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::subagent_executor::get_subagent_executor;
use crate::team_roster;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// How long members get to notice the shutdown request and finish their round.
///
/// A tool round is the unit here, not a wall-clock estimate of the work: the
/// flag is checked at round boundaries, so this is "long enough for a model
/// call plus a tool", not "long enough to finish the task".
const SHUTDOWN_GRACE: Duration = Duration::from_secs(60);

/// How often the roster is re-read while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct TeamDeleteTool {
    project_dir: PathBuf,
    /// How long members get to stop. A field rather than a constant so the
    /// straggler path is testable without a minute of real waiting.
    grace: Duration,
}

impl TeamDeleteTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            project_dir,
            grace: SHUTDOWN_GRACE,
        }
    }

    #[cfg(test)]
    fn with_grace(project_dir: PathBuf, grace: Duration) -> Self {
        Self { project_dir, grace }
    }
}

#[async_trait]
impl Tool for TeamDeleteTool {
    fn name(&self) -> &str {
        "TeamDelete"
    }

    fn description(&self) -> &str {
        "Shut down a team: ask every running member to stop, wait for them to finish \
         their current round, then remove the team. Refuses if a member does not stop, \
         leaving the team intact."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": { "type": "string", "description": "Team ID to delete" }
            },
            "required": ["team_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let team_id = match input["team_id"].as_str() {
            Some(id) => id.trim().to_string(),
            None => return ToolResult::error("missing 'team_id'"),
        };

        let team_dir = team_roster::team_dir(&self.project_dir, &team_id);
        if !team_dir.exists() {
            return ToolResult::error(format!("team '{team_id}' not found"));
        }

        // Only the active team has running members; any other team on disk is a
        // roster with nobody in it, and deleting it is just a directory removal.
        let is_active = team_roster::active().is_some_and(|team| team.team_id == team_id);
        let mut requested: Vec<String> = Vec::new();

        if is_active {
            requested = team_roster::seated_agent_ids();
            if let Some(error) = request_stop(&requested).await {
                return ToolResult::error(error);
            }
            let stragglers = wait_for_departures(self.grace).await;
            if !stragglers.is_empty() {
                return ToolResult::error(format!(
                    "team '{team_id}' left intact: {} member(s) did not stop within {:?}: {}. \
                     They are still running — stop them and try again.",
                    stragglers.len(),
                    self.grace,
                    stragglers.join(", "),
                ));
            }
            team_roster::deactivate();
        }

        if let Err(e) = std::fs::remove_dir_all(&team_dir) {
            // The members are already stopped at this point, so re-activating
            // would claim a team that has nobody on it. Say what happened.
            return ToolResult::error(format!(
                "members stopped, but the team directory could not be removed: {e}"
            ));
        }

        ToolResult::success(
            serde_json::to_string(&json!({
                "deleted": team_id,
                "members_stopped": requested,
            }))
            .unwrap_or_default(),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

/// Send the cooperative stop signal to every seated member.
///
/// `Some(error)` when there is no executor to send through — refusing beats
/// deleting a roster whose members were never told anything.
async fn request_stop(agent_ids: &[String]) -> Option<String> {
    if agent_ids.is_empty() {
        return None;
    }
    let Some(exec) = get_subagent_executor() else {
        return Some(format!(
            "cannot shut down {} running member(s): no subagent executor is installed, \
             so there is no way to signal them",
            agent_ids.len()
        ));
    };

    for id in agent_ids {
        // `false` means the agent is already gone — it left the roster between
        // the read and here, which is the outcome we wanted anyway.
        let running = exec.request_shutdown(id).await;
        tracing::info!(agent_id = %id, running, "team shutdown requested");
    }
    None
}

/// Wait for members to leave the roster, and report whoever did not.
///
/// The roster is the acknowledgement: an agent leaves it from the same terminal
/// hook that reports it complete, so a member still seated is a member still
/// running. There is nothing to infer.
async fn wait_for_departures(grace: Duration) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + grace;
    loop {
        let still_seated = team_roster::seated_agent_ids();
        if still_seated.is_empty() {
            return Vec::new();
        }
        if tokio::time::Instant::now() >= deadline {
            return still_seated;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
#[path = "team_delete_tests.rs"]
mod tests;
