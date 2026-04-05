use serde_json::json;

use crate::agent_tool::SubagentRequest;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that creates a new tracked task in the global TaskManager.
///
/// When a `prompt` field is provided, the response includes a `subagent_request`
/// that the agent loop should use to spawn a background agent for the task.
/// Without `prompt`, the task is created for manual tracking only.
pub struct TaskCreateTool;

#[async_trait::async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }

    fn description(&self) -> &str {
        "Create a new task to track work. Optionally spawns a background agent \
         by providing a prompt. Returns the task ID and, if applicable, a \
         subagent_request for the agent loop to execute."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["subject", "description"],
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short subject/title for the task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of the task"
                },
                "prompt": {
                    "type": "string",
                    "description": "Task prompt for the background agent. If provided, a subagent will be spawned."
                },
                "model": {
                    "type": "string",
                    "description": "Model to use for the subagent (defaults to parent model)"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tools the subagent is allowed to use"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum conversation turns for the subagent (default 10, max 100)"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let subject = match input.get("subject").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: subject"),
        };

        let description = match input.get("description").and_then(|v| v.as_str()) {
            Some(s) => s,
            _ => return ToolResult::error("missing required field: description"),
        };

        let full_desc = format!("{subject}: {description}");
        let task_id = crate::task_manager::TASK_MANAGER.create_task(&full_desc);

        // Build response — always includes task_id
        let mut response = json!({ "task_id": task_id });

        // If prompt is provided, build a SubagentRequest and include it
        if let Some(prompt) = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let model = input
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let allowed_tools = match input.get("allowed_tools") {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => Vec::new(),
            };

            let max_turns = match input.get("max_turns").and_then(|v| v.as_u64()) {
                Some(n) if n == 0 || n > 100 => {
                    return ToolResult::error("max_turns must be between 1 and 100");
                }
                Some(n) => n as u32,
                None => SubagentRequest::DEFAULT_MAX_TURNS,
            };

            let request = SubagentRequest {
                prompt: prompt.to_string(),
                model,
                allowed_tools,
                max_turns,
                timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
            };

            response["subagent_request"] =
                serde_json::to_value(&request).unwrap_or_else(|_| json!(null));
        }

        match serde_json::to_string_pretty(&response) {
            Ok(s) => ToolResult::success(s),
            Err(e) => ToolResult::error(format!("failed to serialize response: {e}")),
        }
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        // If a prompt is provided, this spawns a subagent — that's risky
        if input
            .get("prompt")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty())
        {
            PermissionLevel::Risky
        } else {
            PermissionLevel::Safe
        }
    }
}
