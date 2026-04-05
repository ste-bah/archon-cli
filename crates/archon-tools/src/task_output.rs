use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that reads captured output from a task.
pub struct TaskOutputTool;

#[async_trait::async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn description(&self) -> &str {
        "Read captured output from a task, optionally with offset and limit."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The 8-character task ID"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset to start reading from (default: 0)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of bytes to return"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        match crate::task_manager::TASK_MANAGER.get_output(task_id, offset, limit) {
            Ok(output) => ToolResult::success(output),
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
