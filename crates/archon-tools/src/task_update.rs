use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that updates a task's description or status.
pub struct TaskUpdateTool;

#[async_trait::async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }

    fn description(&self) -> &str {
        "Update a task's description or status."
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
                "description": {
                    "type": "string",
                    "description": "New description for the task"
                },
                "status": {
                    "type": "string",
                    "description": "New status: Pending, Running, Completed, Failed, or Stopped"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        let mgr = &crate::task_manager::TASK_MANAGER;

        // Verify task exists
        if mgr.get_task(task_id).is_none() {
            return ToolResult::error(format!("task not found: {task_id}"));
        }

        // Update description if provided
        let new_desc = input.get("description").and_then(|v| v.as_str());
        if let Some(desc) = new_desc
            && let Err(e) = mgr.update_task(task_id, Some(desc))
        {
            return ToolResult::error(e);
        }

        // Update status if provided
        if let Some(status_str) = input.get("status").and_then(|v| v.as_str()) {
            let status = match status_str {
                "Pending" => crate::task_manager::TaskStatus::Pending,
                "Running" => crate::task_manager::TaskStatus::Running,
                "Completed" => crate::task_manager::TaskStatus::Completed,
                "Failed" => crate::task_manager::TaskStatus::Failed,
                "Stopped" => crate::task_manager::TaskStatus::Stopped,
                other => {
                    return ToolResult::error(format!(
                        "invalid status: '{other}'. Must be Pending, Running, Completed, Failed, or Stopped"
                    ));
                }
            };
            mgr.set_status(task_id, status);
        }

        ToolResult::success(format!("task {task_id} updated"))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
