//! ReadTeamMessages tool for TASK-CLI-312.
//!
//! Reads and clears pending messages from a member's inbox JSONL file.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::team_message::TeamMessage;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct ReadTeamMessagesTool {
    project_dir: PathBuf,
}

impl ReadTeamMessagesTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn inbox_path(&self, team_id: &str, role: &str) -> PathBuf {
        self.project_dir
            .join(".claude")
            .join("teams")
            .join(team_id)
            .join(format!("inbox-{role}.jsonl"))
    }
}

#[async_trait]
impl Tool for ReadTeamMessagesTool {
    fn name(&self) -> &str {
        "ReadTeamMessages"
    }

    fn description(&self) -> &str {
        "Read and clear all pending messages from a team member's inbox."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": { "type": "string" },
                "role": { "type": "string", "description": "The member role reading their inbox" }
            },
            "required": ["team_id", "role"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let team_id = match input["team_id"].as_str() {
            Some(id) => id,
            None => return ToolResult::error("missing 'team_id'"),
        };
        let role = match input["role"].as_str() {
            Some(r) => r,
            None => return ToolResult::error("missing 'role'"),
        };

        let path = self.inbox_path(team_id, role);
        if !path.exists() {
            // Empty inbox is not an error — just return empty
            return ToolResult::success(
                serde_json::to_string(&json!({ "messages": [] })).unwrap_or_default(),
            );
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("failed to read inbox: {e}")),
        };

        let messages: Vec<TeamMessage> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        // Clear the inbox after reading
        if let Err(e) = std::fs::write(&path, "") {
            return ToolResult::error(format!("failed to clear inbox: {e}"));
        }

        ToolResult::success(
            serde_json::to_string_pretty(&json!({ "messages": messages })).unwrap_or_default(),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
