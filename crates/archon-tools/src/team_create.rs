//! `TeamCreate` — establish the session's team (TASK-CLI-312, wired in #184 M5).
//!
//! This used to write a `team.json` nothing read and one empty `inbox-<role>
//! .jsonl` per member that nothing drained, then report success — the shape #153
//! rules out, because a broken subsystem looked healthy. The file mailboxes are
//! gone: every agent shares one process, so member messaging is `SendMessage`
//! through the router, and transcripts already give crash-recoverable delivery.
//!
//! What it does now is establish the team. A spawn while a team is active takes
//! a seat on it, and the roster is what `/agents`, `archon team list` and
//! `TeamDelete` all read.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::team_config::{MemberConfig, TeamConfig};
use crate::team_roster;
use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

pub struct TeamCreateTool {
    project_dir: PathBuf,
}

impl TeamCreateTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }
}

#[async_trait]
impl Tool for TeamCreateTool {
    fn name(&self) -> &str {
        "TeamCreate"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::ControlPlane
    }

    fn description(&self) -> &str {
        "Establish a team of agents for this session. Declares the roles the team wants; \
         spawning an agent whose subagent_type matches a role seats it on the team. \
         Members address each other by role with SendMessage. Does not spawn anything \
         itself — use the Agent tool for that, then TeamDelete to shut the team down."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Human-readable team name" },
                "members": {
                    "type": "array",
                    "description": "The roles this team wants. A role is the subagent_type \
                                    of the agent that fills it, and the address other members \
                                    send to.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "role": { "type": "string" },
                            "system_prompt": { "type": "string" },
                            "model": { "type": "string" },
                            "tools": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["role", "system_prompt"]
                    }
                }
            },
            "required": ["name", "members"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let name = match input["name"].as_str() {
            Some(n) => n.to_string(),
            None => return ToolResult::error("missing 'name'"),
        };
        let members_val = match input["members"].as_array() {
            Some(m) => m,
            None => return ToolResult::error("missing 'members'"),
        };

        // One team per session. Switching while agents are seated would leave
        // them on a roster nothing reads and route their departures into the
        // new team's file.
        if let Some(active) = team_roster::active() {
            let seated = team_roster::seated_agent_ids();
            if !seated.is_empty() {
                return ToolResult::error(format!(
                    "team '{}' is still running {} agent(s): {}. \
                     Shut it down with TeamDelete before creating another.",
                    active.team_id,
                    seated.len(),
                    seated.join(", "),
                ));
            }
        }

        let team_id = uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>();

        let mut members = Vec::new();
        for m in members_val {
            let role = match m["role"].as_str() {
                Some(r) => r.trim().to_string(),
                None => return ToolResult::error("member missing 'role'"),
            };
            if role.is_empty() {
                return ToolResult::error("member 'role' is empty");
            }
            members.push(MemberConfig {
                role,
                system_prompt: m["system_prompt"].as_str().unwrap_or("").to_string(),
                model: m["model"].as_str().map(String::from),
                tools: m["tools"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                agent_id: None,
                declared: true,
            });
        }

        let config = TeamConfig {
            id: team_id.clone(),
            name,
            members,
        };

        if let Err(error) = team_roster::save(&self.project_dir, &config) {
            return ToolResult::error(format!("failed to write the team roster: {error}"));
        }
        team_roster::activate(self.project_dir.clone(), team_id.clone());

        let roles: Vec<&str> = config.members.iter().map(|m| m.role.as_str()).collect();
        ToolResult::success(
            serde_json::to_string_pretty(&json!({
                "team_id": team_id,
                "roles": roles,
                "team_dir": team_roster::team_dir(&self.project_dir, &team_id).to_string_lossy(),
                "next": "Spawn agents with the Agent tool using subagent_type set to a role \
                         above; each one is seated on this team automatically. Members reach \
                         each other by role with SendMessage."
            }))
            .unwrap_or_default(),
        )
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::Arbitrary
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[cfg(test)]
#[path = "team_create_tests.rs"]
mod tests;
