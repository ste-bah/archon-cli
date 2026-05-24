use std::path::Path;

use archon_permissions::auto::AutoDecision;
use archon_permissions::is_default_safe_tool;

use super::tool_preflight_gates::file_path_for_tool;
use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn preflight_single_tool(
        &mut self,
        tool: &PendingToolCall,
        effective_mode: AgentMode,
    ) -> Option<PreflightResult> {
        let (perm_mode, checker_decision, tool_arc, mut input) =
            self.resolve_preflight_tool(tool).await?;
        if !self
            .permission_allows_tool(tool, &input, &perm_mode, checker_decision)
            .await
        {
            return None;
        }
        if !self.plan_mode_allows_tool(tool, effective_mode).await {
            return None;
        }
        if !self.run_pre_tool_hooks(tool, &perm_mode, &mut input).await {
            return None;
        }
        let sandbox_prechecked = self.precheck_sandbox(tool, &input).await?;
        self.snapshot_before_mutation(tool, &input).await;
        let file_path = file_path_for_tool(tool, &input);
        self.fire_before_tool_call_hook(&tool.name, &tool.id, &input)
            .await;

        Some(PreflightResult {
            tool_name: tool.name.clone(),
            tool_id: tool.id.clone(),
            input,
            tool_arc,
            file_path,
            sandbox_prechecked,
        })
    }

    pub(super) async fn deny_preflight_tool(
        &mut self,
        tool: &PendingToolCall,
        mode: &str,
        reason: &str,
    ) {
        self.fire_permission_denied_hook(tool, mode, reason).await;
        {
            let mut log = self.denial_log.lock().await;
            log.record(&tool.name, reason);
        }
        let denied_result = ToolResult::error(format!(
            "Permission denied for tool '{}'. Current mode: {}. Reason: {}",
            tool.name, mode, reason
        ));
        self.send_event(AgentEvent::ToolCallComplete {
            name: tool.name.clone(),
            id: tool.id.clone(),
            result: denied_result.clone(),
        })
        .await;
        self.state
            .add_tool_result(&tool.id, &denied_result.content, true);
    }

    pub(super) async fn handle_unknown_tool_prelookup_hook_denial(
        &mut self,
        tool: &PendingToolCall,
        mode: &str,
    ) -> bool {
        let Some(ref registry) = self.hook_registry else {
            return false;
        };
        let input = serde_json::from_str::<serde_json::Value>(tool.input_json.trim())
            .unwrap_or_else(|_| serde_json::json!({}));
        let hook_agg = registry
            .execute_hooks(
                crate::hooks::HookEvent::PreToolUse,
                serde_json::json!({
                    "hook_event": "PreToolUse",
                    "tool_name": tool.name,
                    "tool_input": input,
                }),
                &self.config.working_dir,
                &self.config.session_id,
            )
            .await;

        if hook_agg.is_blocked() {
            let reason = hook_agg
                .block_reason()
                .unwrap_or_else(|| "hook blocked".to_owned());
            let result = ToolResult::error(format!("Hook blocked: {reason}"));
            self.send_event(AgentEvent::ToolCallComplete {
                name: tool.name.clone(),
                id: tool.id.clone(),
                result: result.clone(),
            })
            .await;
            self.state
                .add_tool_result(&tool.id, &result.content, result.is_error);
            return true;
        }

        if matches!(
            hook_agg.permission_behavior,
            Some(crate::hooks::PermissionBehavior::Deny)
        ) {
            let reason = hook_agg
                .permission_decision_reason
                .as_deref()
                .unwrap_or("hook denied permission")
                .to_string();
            self.deny_preflight_tool(tool, mode, &reason).await;
            return true;
        }

        false
    }

    pub(super) async fn auto_mode_tool_allowed(
        &self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
    ) -> bool {
        let Some(ref evaluator) = self.auto_evaluator else {
            return true;
        };
        let decision = match tool.name.as_str() {
            "Bash" | "PowerShell" => {
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
                evaluator.evaluate_command(cmd)
            }
            "Write" | "Edit" => {
                let path = input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                evaluator.evaluate_file_write(Path::new(path))
            }
            "TodoWrite" | "Sleep" => AutoDecision::Allow,
            _ if is_default_safe_tool(&tool.name) => AutoDecision::Allow,
            "Config" => {
                let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action.eq_ignore_ascii_case("get") {
                    AutoDecision::Allow
                } else {
                    AutoDecision::Prompt
                }
            }
            _ => AutoDecision::Prompt,
        };
        match decision {
            AutoDecision::Allow => {
                tracing::debug!(tool = %tool.name, "auto-mode: allowed");
                true
            }
            AutoDecision::Prompt => {
                tracing::warn!(tool = %tool.name, "auto-mode: risky, denied");
                self.fire_permission_denied_hook(tool, "auto", "risky_operation")
                    .await;
                false
            }
            AutoDecision::PromptWithWarning(msg) => {
                tracing::warn!(tool = %tool.name, warning = %msg, "auto-mode: dangerous, denied");
                self.fire_hook(
                    crate::hooks::HookEvent::PermissionDenied,
                    serde_json::json!({
                        "hook_event": "PermissionDenied",
                        "tool_name": tool.name,
                        "mode": "auto",
                        "reason": "dangerous_operation",
                        "warning": msg,
                    }),
                )
                .await;
                self.send_event(AgentEvent::PermissionDenied {
                    tool: tool.name.clone(),
                    reason: Some("dangerous_operation".to_string()),
                })
                .await;
                false
            }
        }
    }
}
