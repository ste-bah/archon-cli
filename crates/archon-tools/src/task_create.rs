use serde_json::json;

use crate::agent_tool::{SubagentRequest, run_subagent_foreground, run_subagent_with_completion};
use crate::subagent_executor::{SubagentClassification, SubagentOutcome, get_subagent_executor};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that creates a new tracked task in the global TaskManager.
///
/// When a `prompt` field is provided, the installed subagent executor runs or
/// spawns the request directly. Without `prompt`, the task is created for manual
/// tracking only.
pub struct TaskCreateTool;

#[async_trait::async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }

    fn description(&self) -> &str {
        "Create a new task to track work. Optionally runs or spawns a subagent \
         by providing a prompt. Returns the task ID and execution result or \
         spawn status when applicable."
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
        let task_id = crate::task_manager::TASK_MANAGER
            .create_task_with_parent(&full_desc, ctx.cancel_parent.as_ref());

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
            Some(n) if n == 0 || n > u64::from(SubagentRequest::MAX_TURNS_HARD_CAP) => {
                return ToolResult::error(format!(
                    "max_turns must be between 1 and {}",
                    SubagentRequest::MAX_TURNS_HARD_CAP
                ));
            }
            Some(n) => {
                tracing::warn!(
                    value = n,
                    tool = "TaskCreate",
                    "max_turns emitted by model despite schema removal -- investigate"
                );
                n as u32
            }
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
            provider_env: None,
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

        // Record the task -> subagent link before dispatching, so `/tasks` can
        // name the agent doing the work. Liveness does not come from here — the
        // runners register every subagent in `BACKGROUND_AGENTS` themselves
        // (`agent_tool::run`), which is what the board claim leases read.
        crate::task_manager::TASK_MANAGER.set_agent_id(&task_id, &subagent_id);

        // Mirror the dispatch onto the run's task board, so delegated work is
        // visible to anyone watching the run rather than only to `/tasks` in
        // this process. Best-effort by design: `raise_delegated_task` returns
        // `None` when no memory service is open, which is the normal case for
        // most test registries, and dispatch carries on either way. The item is
        // closed out from `TASK_MANAGER::set_status`, which every terminal
        // transition below already goes through.
        if let Some(item_id) = crate::board::raise_delegated_task(
            &ctx.session_id,
            &subagent_id,
            &full_desc,
            prompt,
            &crate::board::caller_id(ctx),
        ) {
            crate::task_manager::TASK_MANAGER.set_board_item_id(&task_id, &item_id);
        }

        // Nested ToolContext: inherit caller's ctx, flip nested=true.
        let nested_ctx = ToolContext {
            nested: true,
            ..ctx.clone()
        };

        // Bug-fix 2026-05-12: previously TASK_MANAGER.create_task() created
        // the task as Pending and the spawn paths below never called
        // set_status. Tasks remained Pending forever. We now transition
        // Pending → Running on dispatch, and Running → Completed/Failed/
        // Stopped on terminal outcome. Auto-backgrounded foreground runs keep
        // a TaskCreate-owned completion receiver that applies terminal status.
        use crate::task_manager::{TASK_MANAGER, TaskStatus};

        fn map_outcome_to_status(outcome: &SubagentOutcome) -> Option<TaskStatus> {
            match outcome {
                SubagentOutcome::Completed(_) => Some(TaskStatus::Completed),
                SubagentOutcome::Failed(_) => Some(TaskStatus::Failed),
                SubagentOutcome::Cancelled => Some(TaskStatus::Stopped),
                // Inner runner keeps executing in a detached task. The
                // TaskCreate-owned completion receiver applies final status.
                SubagentOutcome::AutoBackgrounded => None,
            }
        }

        match exec.classify(&request) {
            SubagentClassification::ExplicitBackground => {
                // Transition Pending → Running synchronously so the response
                // we hand back reflects the dispatched state.
                TASK_MANAGER.set_status(&task_id, TaskStatus::Running);

                // Spawn detached. The closure owns task_id so it can update
                // TASK_MANAGER when run_subagent returns.
                let sid_spawn = subagent_id.clone();
                let cancel = TASK_MANAGER
                    .execution_token(&task_id)
                    .expect("new task has an execution token");
                let ctx_spawn = nested_ctx.clone();
                let task_id_spawn = task_id.clone();
                archon_observability::spawn_named("task-create-subagent-background", async move {
                    let outcome =
                        run_subagent_foreground(sid_spawn, request, cancel, ctx_spawn).await;
                    if let Some(final_status) = map_outcome_to_status(&outcome) {
                        TASK_MANAGER.set_status(&task_id_spawn, final_status);
                    }
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
                // Transition Pending → Running before awaiting the runner so
                // any concurrent /tasks query sees Running, not Pending.
                TASK_MANAGER.set_status(&task_id, TaskStatus::Running);

                let cancel = TASK_MANAGER
                    .execution_token(&task_id)
                    .expect("new task has an execution token");
                let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
                let outcome = run_subagent_with_completion(
                    subagent_id.clone(),
                    request,
                    cancel,
                    nested_ctx,
                    completion_tx,
                )
                .await;
                if let Some(final_status) = map_outcome_to_status(&outcome) {
                    TASK_MANAGER.set_status(&task_id, final_status);
                } else {
                    let task_id_completion = task_id.clone();
                    archon_observability::spawn_named(
                        "task-create-auto-background-completion",
                        async move {
                            let final_status = completion_rx
                                .await
                                .ok()
                                .and_then(|outcome| map_outcome_to_status(&outcome))
                                .unwrap_or(TaskStatus::Failed);
                            TASK_MANAGER.set_status(&task_id_completion, final_status);
                        },
                    );
                }
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

    #[tokio::test]
    async fn task_create_schema_does_not_expose_max_turns() {
        let tool = TaskCreateTool;
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().expect("schema properties");
        assert!(
            !props.contains_key("max_turns"),
            "TaskCreate schema must not advertise max_turns"
        );
    }

    // Prompt-path routing is covered in tests/task_ags_105.rs with an installed
    // recording executor. This module keeps manual-task and schema contracts.

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
