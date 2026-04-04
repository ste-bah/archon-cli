use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that stops a running task by setting its cancellation token.
pub struct TaskStopTool;

#[async_trait::async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }

    fn description(&self) -> &str {
        "Stop a running task by setting its cancellation token."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The 8-character task ID to stop"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        match crate::task_manager::TASK_MANAGER.stop_task(task_id) {
            Ok(()) => ToolResult::success(format!("task {task_id} stopped")),
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
