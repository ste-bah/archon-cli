//! TeamCreate tool for TASK-CLI-312.
//!
//! Creates a team config file and per-member inbox files.
//! Does NOT spawn agent processes — returns config for the caller to use.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::team_config::{MemberConfig, TeamConfig};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

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

    fn description(&self) -> &str {
        "Create a new agent team. Writes team.json and per-member inbox files. \
         Does NOT spawn agent processes — returns team ID and role names for the caller to use."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Human-readable team name" },
                "members": {
                    "type": "array",
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

        let team_id = uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>();

        let mut members = Vec::new();
        for m in members_val {
            let role = match m["role"].as_str() {
                Some(r) => r.to_string(),
                None => return ToolResult::error("member missing 'role'"),
            };
            let system_prompt = m["system_prompt"].as_str().unwrap_or("").to_string();
            let model = m["model"].as_str().map(String::from);
            let tools: Vec<String> = m["tools"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            members.push(MemberConfig {
                role,
                system_prompt,
                model,
                tools,
            });
        }

        let config = TeamConfig {
            id: team_id.clone(),
            name,
            members: members.clone(),
        };

        let teams_dir = self.project_dir.join(".claude").join("teams");
        let team_dir = teams_dir.join(&team_id);
        if let Err(e) = std::fs::create_dir_all(&team_dir) {
            return ToolResult::error(format!("failed to create team directory: {e}"));
        }

        // Write team.json
        let json_str = match serde_json::to_string_pretty(&config) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("serialization error: {e}")),
        };
        if let Err(e) = std::fs::write(team_dir.join("team.json"), &json_str) {
            return ToolResult::error(format!("failed to write team.json: {e}"));
        }

        // Create empty inbox files for each member
        for member in &members {
            let inbox_path = team_dir.join(format!("inbox-{}.jsonl", member.role));
            if let Err(e) = std::fs::write(&inbox_path, "") {
                return ToolResult::error(format!(
                    "failed to create inbox for '{}': {e}",
                    member.role
                ));
            }
        }

        let roles: Vec<&str> = members.iter().map(|m| m.role.as_str()).collect();
        ToolResult::success(
            serde_json::to_string_pretty(&json!({
                "team_id": team_id,
                "roles": roles,
                "team_dir": team_dir.to_string_lossy()
            }))
            .unwrap_or_default(),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
