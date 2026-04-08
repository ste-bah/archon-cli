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

        let new_team_dir = self.project_dir.join(".archon").join("teams").join(team_id);
        let team_dir = if new_team_dir.exists() {
            new_team_dir
        } else {
            let old_team_dir = self.project_dir.join(".claude").join("teams").join(team_id);
            if old_team_dir.exists() {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_team_dir.display(),
                    new_team_dir.display()
                );
                old_team_dir
            } else {
                new_team_dir
            }
        };

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
