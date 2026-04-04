use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that retrieves detailed information about a task.
pub struct TaskGetTool;

#[async_trait::async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }

    fn description(&self) -> &str {
        "Get detailed information about a task by its ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The 8-character task ID"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        match crate::task_manager::TASK_MANAGER.get_task(task_id) {
            Some(info) => match serde_json::to_string_pretty(&info) {
                Ok(s) => ToolResult::success(s),
                Err(e) => ToolResult::error(format!("failed to serialize task info: {e}")),
            },
            None => ToolResult::error(format!("task not found: {task_id}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
