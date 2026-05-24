use std::sync::Arc;

use archon_permissions::mode::{PermissionDecision, PermissionMode};
use archon_tools::plan_mode::is_tool_allowed_in_mode;
use archon_tools::tool::Tool;

use super::*;

impl Agent {
    pub(super) async fn resolve_preflight_tool(
        &mut self,
        tool: &PendingToolCall,
    ) -> Option<(String, PermissionDecision, Arc<dyn Tool>, serde_json::Value)> {
        let perm_mode = self.config.permission_mode.lock().await.clone();
        let description = format!("use {}", tool.name);
        let checker_decision = self.permission_checker_decision(
            &perm_mode,
            &tool.name,
            &tool.input_json,
            &description,
        );
        if let PermissionDecision::Deny(reason) = &checker_decision {
            self.deny_preflight_tool(tool, &perm_mode, reason).await;
            return None;
        }

        let tool_arc = match self.registry.lookup(&tool.name) {
            Some(t) => t,
            None => {
                if self
                    .handle_unknown_tool_prelookup_hook_denial(tool, &perm_mode)
                    .await
                {
                    return None;
                }
                let result = ToolResult::error(format!(
                    "Unknown tool: '{}'. Available tools: {}",
                    tool.name,
                    self.registry.tool_names().join(", ")
                ));
                self.send_event(AgentEvent::ToolCallComplete {
                    name: tool.name.clone(),
                    id: tool.id.clone(),
                    result: result.clone(),
                })
                .await;
                self.state.add_tool_result(&tool.id, &result.content, true);
                return None;
            }
        };

        let allow_empty = tool_input_json::schema_allows_empty_input(&tool_arc.input_schema());
        let input = match tool_input_json::parse_pending_tool_input(
            &tool.name,
            &tool.id,
            &tool.input_json,
            allow_empty,
        ) {
            Ok(input) => input,
            Err(err) => {
                tracing::warn!(
                    tool = %tool.name,
                    tool_use_id = %tool.id,
                    input_len = tool.input_json.len(),
                    "{err}"
                );
                let result = ToolResult::error(err);
                self.send_event(AgentEvent::ToolCallComplete {
                    name: tool.name.clone(),
                    id: tool.id.clone(),
                    result: result.clone(),
                })
                .await;
                self.state
                    .add_tool_result(&tool.id, &result.content, result.is_error);
                return None;
            }
        };

        Some((perm_mode, checker_decision, tool_arc, input))
    }

    pub(super) async fn permission_allows_tool(
        &mut self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
        perm_mode: &str,
        checker_decision: PermissionDecision,
    ) -> bool {
        let parsed_mode = perm_mode.parse::<PermissionMode>().unwrap_or_default();
        let mut denial_reason = format!("mode={perm_mode}");
        let tool_allowed = match checker_decision {
            PermissionDecision::Allow => {
                tracing::debug!(tool = %tool.name, mode = %perm_mode, "permission checker allowed");
                true
            }
            PermissionDecision::NeedsPermission(reason) => {
                denial_reason = reason.clone();
                if parsed_mode == PermissionMode::Auto {
                    self.auto_mode_tool_allowed(tool, input).await
                } else {
                    self.request_tool_permission(tool, perm_mode, reason).await
                }
            }
            PermissionDecision::Deny(reason) => {
                self.deny_preflight_tool(tool, perm_mode, &reason).await;
                return false;
            }
        };
        if !tool_allowed {
            self.record_preflight_denial(tool, perm_mode, &denial_reason)
                .await;
        }
        tool_allowed
    }

    pub(super) async fn plan_mode_allows_tool(
        &mut self,
        tool: &PendingToolCall,
        effective_mode: AgentMode,
    ) -> bool {
        if is_tool_allowed_in_mode(&tool.name, effective_mode) {
            return true;
        }
        let result = ToolResult::error(format!(
            "Tool '{}' is not available in plan mode. Only read-only tools are allowed.",
            tool.name
        ));
        self.send_event(AgentEvent::ToolCallComplete {
            name: tool.name.clone(),
            id: tool.id.clone(),
            result: result.clone(),
        })
        .await;
        self.state.add_tool_result(&tool.id, &result.content, true);
        false
    }

    pub(super) async fn run_pre_tool_hooks(
        &mut self,
        tool: &PendingToolCall,
        perm_mode: &str,
        input: &mut serde_json::Value,
    ) -> bool {
        let Some(ref registry) = self.hook_registry else {
            return true;
        };
        let hook_agg = registry
            .execute_hooks(
                crate::hooks::HookEvent::PreToolUse,
                serde_json::json!({
                    "hook_event": "PreToolUse",
                    "tool_name": tool.name,
                    "tool_input": input.clone(),
                }),
                &self.config.working_dir,
                &self.config.session_id,
            )
            .await;
        if self.blocked_by_pre_tool_hook(tool, &hook_agg).await {
            return false;
        }
        if !self
            .apply_pre_tool_permission_behavior(tool, perm_mode, &hook_agg)
            .await
        {
            return false;
        }
        self.apply_pre_tool_input_update(tool, input, hook_agg);
        true
    }

    pub(super) async fn precheck_sandbox(
        &mut self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
    ) -> Option<bool> {
        match self
            .config
            .sandbox
            .as_ref()
            .map(|backend| backend.check(&tool.name, input))
        {
            Some(Ok(())) => Some(true),
            Some(Err(reason)) => {
                let result =
                    ToolResult::error(format!("Sandbox denied tool '{}': {reason}", tool.name));
                self.send_event(AgentEvent::ToolCallComplete {
                    name: tool.name.clone(),
                    id: tool.id.clone(),
                    result: result.clone(),
                })
                .await;
                self.state.add_tool_result(&tool.id, &result.content, true);
                None
            }
            None => Some(false),
        }
    }

    pub(super) async fn snapshot_before_mutation(
        &self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
    ) {
        if matches!(tool.name.as_str(), "Write" | "Edit")
            && let Some(ref store) = self.checkpoint_store
            && let Some(file_path) = input.get("file_path").and_then(|v| v.as_str())
        {
            let store = store.lock().await;
            if let Err(e) = store.snapshot(
                &self.config.session_id,
                file_path,
                self.turn_number as i64,
                &tool.name,
            ) {
                tracing::warn!("checkpoint snapshot failed for {file_path}: {e}");
            }
        }
    }

    async fn record_preflight_denial(
        &mut self,
        tool: &PendingToolCall,
        perm_mode: &str,
        denial_reason: &str,
    ) {
        {
            let mut log = self.denial_log.lock().await;
            log.record(&tool.name, denial_reason);
        }
        let denied_result = ToolResult::error(format!(
            "Permission denied for tool '{}'. Current mode: {}. Reason: {}",
            tool.name, perm_mode, denial_reason
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

    async fn blocked_by_pre_tool_hook(
        &mut self,
        tool: &PendingToolCall,
        hook_agg: &crate::hooks::AggregatedHookResult,
    ) -> bool {
        if !hook_agg.is_blocked() {
            return false;
        }
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
        true
    }

    async fn apply_pre_tool_permission_behavior(
        &mut self,
        tool: &PendingToolCall,
        perm_mode: &str,
        hook_agg: &crate::hooks::AggregatedHookResult,
    ) -> bool {
        match hook_agg.permission_behavior {
            Some(crate::hooks::PermissionBehavior::Deny) => {
                let reason = hook_agg
                    .permission_decision_reason
                    .as_deref()
                    .unwrap_or("hook denied permission");
                self.fire_permission_denied_hook(tool, perm_mode, reason)
                    .await;
                self.record_preflight_hook_denial(tool, reason).await;
                false
            }
            Some(crate::hooks::PermissionBehavior::Allow) => {
                tracing::debug!(tool = %tool.name, "permission overridden to Allow by policy hook");
                true
            }
            Some(crate::hooks::PermissionBehavior::Ask) => {
                tracing::debug!(
                    tool = %tool.name,
                    "permission_behavior=ask (not yet implemented, using normal flow)"
                );
                true
            }
            Some(crate::hooks::PermissionBehavior::Passthrough) | None => true,
        }
    }

    async fn record_preflight_hook_denial(&mut self, tool: &PendingToolCall, reason: &str) {
        {
            let mut log = self.denial_log.lock().await;
            log.record(&tool.name, reason);
        }
        let result = ToolResult::error(format!("Permission denied: {reason}"));
        self.send_event(AgentEvent::ToolCallComplete {
            name: tool.name.clone(),
            id: tool.id.clone(),
            result: result.clone(),
        })
        .await;
        self.state
            .add_tool_result(&tool.id, &result.content, result.is_error);
    }

    fn apply_pre_tool_input_update(
        &self,
        tool: &PendingToolCall,
        input: &mut serde_json::Value,
        hook_agg: crate::hooks::AggregatedHookResult,
    ) {
        if let Some(modified_input) = hook_agg.updated_input {
            if modified_input.is_object() {
                tracing::debug!(tool = %tool.name, "PreToolUse hook modified tool input");
                *input = modified_input;
            } else {
                tracing::warn!(
                    tool = %tool.name,
                    "PreToolUse hook returned non-object updated_input, ignoring"
                );
            }
        }
        for msg in &hook_agg.system_messages {
            tracing::warn!(tool = %tool.name, "[Hook Warning] {}", msg);
        }
        for msg in &hook_agg.status_messages {
            tracing::info!(tool = %tool.name, "[Hook Status] {}", msg);
        }
    }
}

pub(super) fn file_path_for_tool(
    tool: &PendingToolCall,
    input: &serde_json::Value,
) -> Option<String> {
    if matches!(tool.name.as_str(), "Write" | "Edit" | "NotebookEdit") {
        input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(String::from)
    } else {
        None
    }
}
