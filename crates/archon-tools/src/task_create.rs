use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::agent_tool::{SubagentRequest, run_subagent};
use crate::subagent_executor::{SubagentClassification, SubagentOutcome, get_subagent_executor};
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
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Optional agent type name for the spawned subagent"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, the spawned subagent runs in the background"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory override for the spawned subagent"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
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

        // Manual task (no prompt): return task_id only.
        let Some(prompt) = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            let response = json!({ "task_id": task_id });
            return match serde_json::to_string_pretty(&response) {
                Ok(s) => ToolResult::success(s),
                Err(e) => ToolResult::error(format!("failed to serialize response: {e}")),
            };
        };

        // Parse SubagentRequest fields (same shape as the old body).
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

        let subagent_type = input
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cwd = input
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let request = SubagentRequest {
            prompt: prompt.to_string(),
            model,
            allowed_tools,
            max_turns,
            timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
            subagent_type,
            run_in_background,
            cwd,
            isolation: None,
        };

        // TASK-AGS-105 Section 2h: route through SubagentExecutor with
        // nested=true so TaskCompleted hook gating (old H5/H9) still fires.
        let exec = match get_subagent_executor() {
            Some(e) => e,
            None => {
                return ToolResult::error(
                    "subagent executor not installed (TaskCreate requires runtime init)",
                );
            }
        };

        // Pre-allocate subagent id (mirror AgentTool::execute's authoritative id).
        let subagent_id = uuid::Uuid::new_v4().to_string();

        // Nested ToolContext: inherit caller's ctx, flip nested=true.
        let nested_ctx = ToolContext {
            nested: true,
            ..ctx.clone()
        };

        match exec.classify(&request) {
            SubagentClassification::ExplicitBackground => {
                // Spawn detached and return task_id + spawn marker synchronously.
                let sid_spawn = subagent_id.clone();
                let cancel = CancellationToken::new();
                let ctx_spawn = nested_ctx.clone();
                tokio::spawn(async move {
                    let _ = run_subagent(sid_spawn, request, cancel, ctx_spawn).await;
                });
                let response = json!({
                    "task_id": task_id,
                    "agent_id": subagent_id,
                    "status": "spawned",
                });
                match serde_json::to_string_pretty(&response) {
                    Ok(s) => ToolResult::success(s),
                    Err(e) => ToolResult::error(format!("failed to serialize response: {e}")),
                }
            }
            SubagentClassification::Foreground => {
                let cancel = CancellationToken::new();
                let outcome = run_subagent(subagent_id.clone(), request, cancel, nested_ctx).await;
                match outcome {
                    SubagentOutcome::Completed(text) => {
                        let response = json!({ "task_id": task_id, "result": text });
                        match serde_json::to_string_pretty(&response) {
                            Ok(s) => ToolResult::success(s),
                            Err(e) => {
                                ToolResult::error(format!("failed to serialize response: {e}"))
                            }
                        }
                    }
                    SubagentOutcome::Failed(err) => ToolResult::error(err),
                    SubagentOutcome::AutoBackgrounded => {
                        let response = json!({
                            "task_id": task_id,
                            "agent_id": subagent_id,
                            "status": "auto_backgrounded",
                        });
                        match serde_json::to_string_pretty(&response) {
                            Ok(s) => ToolResult::success(s),
                            Err(e) => {
                                ToolResult::error(format!("failed to serialize response: {e}"))
                            }
                        }
                    }
                    SubagentOutcome::Cancelled => ToolResult::error("subagent cancelled"),
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test-session".into(),
            mode: crate::tool::AgentMode::Normal,
            extra_dirs: vec![],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn schema_includes_subagent_request_fields() {
        let tool = TaskCreateTool;
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().expect("schema properties");

        assert!(props.contains_key("subagent_type"));
        assert!(props.contains_key("run_in_background"));
        assert!(props.contains_key("cwd"));
    }

    // TASK-AGS-105 Section 8 test rewrite: the old tests asserted
    // `response["subagent_request"]` shape, which was a serialized
    // SubagentRequest passed back through the dispatch loop for re-entry
    // into handle_subagent_result. That indirection is deleted. TaskCreate
    // now routes directly through the SubagentExecutor and returns either
    // a spawn marker (background) or the final result (foreground).
    //
    // Because TaskCreate now needs an executor installed to run the
    // prompt path, the prompted tests require either an installed
    // executor (not portable) or the manual-task early return path.
    // The manual-task path (no prompt) still exercises task creation +
    // serialization and does not touch the executor seam.

    #[tokio::test]
    async fn execute_manual_task_returns_task_id_only() {
        let tool = TaskCreateTool;
        let input = json!({
            "subject": "Review",
            "description": "Review manually without a subagent"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);

        let response: serde_json::Value =
            serde_json::from_str(&result.content).expect("response json");
        assert!(response["task_id"].is_string(), "must contain task_id");
        assert!(
            response.get("subagent_request").is_none(),
            "TASK-AGS-105: serialized subagent_request no longer emitted"
        );
        assert!(
            response.get("agent_id").is_none(),
            "manual task path must not spawn a subagent"
        );
    }
}
