//! CronDelete tool for TASK-CLI-311.

use std::path::PathBuf;

use serde_json::json;

use crate::cron_task::CronStore;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that deletes a scheduled cron task by ID.
pub struct CronDeleteTool {
    project_dir: PathBuf,
}

impl CronDeleteTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn store(&self) -> CronStore {
        let new_path = self
            .project_dir
            .join(".archon")
            .join("scheduled_tasks.json");
        if new_path.exists() {
            return CronStore::new(new_path);
        }
        let old_path = self
            .project_dir
            .join(".claude")
            .join("scheduled_tasks.json");
        if old_path.exists() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_path.display(),
                new_path.display()
            );
            return CronStore::new(old_path);
        }
        CronStore::new(new_path)
    }
}

#[async_trait::async_trait]
impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }

    fn description(&self) -> &str {
        "Delete a scheduled cron task by its ID. The task will no longer fire after deletion."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Task ID to delete (from CronCreate or CronList)."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let id = match input.get("id").and_then(|v| v.as_str()) {
            Some(i) if !i.is_empty() => i.to_owned(),
            _ => return ToolResult::error("CronDelete: 'id' is required"),
        };

        let store = self.store();
        if let Err(e) = store.delete_required(&id) {
            return ToolResult::error(format!("{e}"));
        }

        ToolResult::success(format!("Deleted cron task '{id}'."))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
