use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that lists all tracked tasks.
pub struct TaskListTool;

#[async_trait::async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }

    fn description(&self) -> &str {
        "List all tracked tasks with their current status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let tasks = crate::task_manager::TASK_MANAGER.list_tasks();

        let summaries: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "description": t.description,
                    "status": t.status,
                    "created_at": t.created_at.to_rfc3339(),
                    "cost": t.cost,
                })
            })
            .collect();

        match serde_json::to_string_pretty(&summaries) {
            Ok(s) => ToolResult::success(s),
            Err(e) => ToolResult::error(format!("failed to serialize task list: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
