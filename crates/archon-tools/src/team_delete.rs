//! TeamDelete tool for TASK-CLI-312.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct TeamDeleteTool {
    project_dir: PathBuf,
}

impl TeamDeleteTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }
}

#[async_trait]
impl Tool for TeamDeleteTool {
    fn name(&self) -> &str {
        "TeamDelete"
    }

    fn description(&self) -> &str {
        "Delete a team and all associated inbox files."
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
            Some(id) => id,
            None => return ToolResult::error("missing 'team_id'"),
        };

        let team_dir = self
            .project_dir
            .join(".claude")
            .join("teams")
            .join(team_id);

        if !team_dir.exists() {
            return ToolResult::error(format!("team '{}' not found", team_id));
        }

        if let Err(e) = std::fs::remove_dir_all(&team_dir) {
            return ToolResult::error(format!("failed to delete team directory: {e}"));
        }

        ToolResult::success(
            serde_json::to_string(&json!({ "deleted": team_id })).unwrap_or_default(),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
