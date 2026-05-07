use std::path::Path;

use archon_permissions::auto::AutoDecision;
use archon_permissions::is_default_safe_tool;
use archon_tools::plan_mode::is_tool_allowed_in_mode;

use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn preflight_tools(
        &mut self,
        pending_tools: &[PendingToolCall],
        effective_mode: AgentMode,
    ) -> Vec<PreflightResult> {
        let mut allowed: Vec<PreflightResult> = Vec::new();

        for tool in pending_tools {
            let mut input: serde_json::Value =
                serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));

            // --- Permission check ---
            let perm_mode = {
                let mode = self.config.permission_mode.lock().await;
                mode.clone()
            };
            let tool_allowed = match perm_mode.as_str() {
                "bypassPermissions" | "yolo" | "dontAsk" => {
                    tracing::debug!(tool = %tool.name, "bypass-mode: allowed");
                    true
                }
                "acceptEdits" => match tool.name.as_str() {
                    "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion" | "TodoWrite"
                    | "Sleep" | "Write" | "Edit" | "Config" | "EnterPlanMode" | "ExitPlanMode"
                    | "NotebookEdit" => true,
                    _ => {
                        let perm_agg = self
                            .fire_hook(
                                crate::hooks::HookEvent::PermissionRequest,
                                serde_json::json!({
                                    "hook_event": "PermissionRequest",
                                    "tool_name": tool.name,
                                    "mode": "acceptEdits",
                                }),
                            )
                            .await;
                        // Apply updated_permissions from hooks (REQ-HOOK-016)
                        if !perm_agg.updated_permissions.is_empty() {
                            let authority = crate::hooks::SourceAuthority::Project;
                            let errors = crate::hooks::apply_permission_updates(
                                &perm_agg.updated_permissions,
                                &authority,
                                self.permission_store.as_ref(),
                            );
                            for err in &errors {
                                tracing::error!("permission update failed: {}", err);
                            }
                        }
                        self.send_event(AgentEvent::PermissionRequired {
                            tool: tool.name.clone(),
                            description: format!("Permission required for {}", tool.name),
                        })
                        .await;
                        self.fire_hook(
                            crate::hooks::HookEvent::PermissionDenied,
                            serde_json::json!({
                                "hook_event": "PermissionDenied",
                                "tool_name": tool.name,
                                "mode": "acceptEdits",
                            }),
                        )
                        .await;
                        self.send_event(AgentEvent::PermissionDenied {
                            tool: tool.name.clone(),
                        })
                        .await;
                        false
                    }
                },
                "default" | "ask" => {
                    if is_default_safe_tool(&tool.name) {
                        tracing::debug!(tool = %tool.name, "default-mode: safe, allowed");
                        true
                    } else {
                        let perm_agg = self
                            .fire_hook(
                                crate::hooks::HookEvent::PermissionRequest,
                                serde_json::json!({
                                    "hook_event": "PermissionRequest",
                                    "tool_name": tool.name,
                                    "mode": "ask",
                                }),
                            )
                            .await;
                        // Apply updated_permissions from hooks (REQ-HOOK-016)
                        if !perm_agg.updated_permissions.is_empty() {
                            let authority = crate::hooks::SourceAuthority::Project;
                            let errors = crate::hooks::apply_permission_updates(
                                &perm_agg.updated_permissions,
                                &authority,
                                self.permission_store.as_ref(),
                            );
                            for err in &errors {
                                tracing::error!("permission update failed: {}", err);
                            }
                        }
                        self.send_event(AgentEvent::PermissionRequired {
                            tool: tool.name.clone(),
                            description: format!("{} wants to use {}", tool.name, tool.name),
                        })
                        .await;

                        if let Some(ref rx) = self.permission_response_rx {
                            let mut rx = rx.lock().await;
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                rx.recv(),
                            )
                            .await
                            {
                                Ok(Some(true)) => {
                                    self.send_event(AgentEvent::PermissionGranted {
                                        tool: tool.name.clone(),
                                    })
                                    .await;
                                    tracing::info!(tool = %tool.name, "default-mode: user approved");
                                    true
                                }
                                _ => {
                                    self.fire_hook(
                                        crate::hooks::HookEvent::PermissionDenied,
                                        serde_json::json!({
                                            "hook_event": "PermissionDenied",
                                            "tool_name": tool.name,
                                            "mode": "ask",
                                            "reason": "user_denied_or_timeout",
                                        }),
                                    )
                                    .await;
                                    self.send_event(AgentEvent::PermissionDenied {
                                        tool: tool.name.clone(),
                                    })
                                    .await;
                                    tracing::info!(tool = %tool.name, "default-mode: user denied or timeout");
                                    false
                                }
                            }
                        } else {
                            tracing::info!(tool = %tool.name, "default-mode: no permission channel, auto-approved");
                            true
                        }
                    }
                }
                _ => {
                    // "auto" mode -- use AutoModeEvaluator
                    if let Some(ref evaluator) = self.auto_evaluator {
                        let decision = match tool.name.as_str() {
                            "Bash" | "PowerShell" => {
                                let cmd =
                                    input.get("command").and_then(|v| v.as_str()).unwrap_or("");
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
                                let action =
                                    input.get("action").and_then(|v| v.as_str()).unwrap_or("");
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
                                self.fire_hook(
                                    crate::hooks::HookEvent::PermissionDenied,
                                    serde_json::json!({
                                        "hook_event": "PermissionDenied",
                                        "tool_name": tool.name,
                                        "mode": "auto",
                                        "reason": "risky_operation",
                                    }),
                                )
                                .await;
                                self.send_event(AgentEvent::PermissionDenied {
                                    tool: tool.name.clone(),
                                })
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
                                })
                                .await;
                                false
                            }
                        }
                    } else {
                        true // no evaluator = allow
                    }
                }
            };

            if !tool_allowed {
                {
                    let mut log = self.denial_log.lock().await;
                    log.record(&tool.name, &format!("mode={perm_mode}"));
                }
                let denied_result = ToolResult::error(format!(
                    "Permission denied for tool '{}'. Current mode: {}. Use /permissions yolo to allow all operations.",
                    tool.name, perm_mode
                ));
                self.send_event(AgentEvent::ToolCallComplete {
                    name: tool.name.clone(),
                    id: tool.id.clone(),
                    result: denied_result.clone(),
                })
                .await;
                self.state
                    .add_tool_result(&tool.id, &denied_result.content, true);
                continue;
            }

            // --- Plan mode check ---
            if !is_tool_allowed_in_mode(&tool.name, effective_mode) {
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
                continue;
            }

            // --- Checkpoint before Write/Edit ---
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

            // --- Pre-tool-use hook (REQ-HOOK-001/003/004) ---
            if let Some(ref registry) = self.hook_registry {
                let hook_input = serde_json::json!({
                    "hook_event": "PreToolUse",
                    "tool_name": tool.name,
                    "tool_input": input,
                });
                let hook_agg = registry
                    .execute_hooks(
                        crate::hooks::HookEvent::PreToolUse,
                        hook_input,
                        &self.config.working_dir,
                        &self.config.session_id,
                    )
                    .await;

                // Check for blocking (any hook returned exit 2 or outcome=Blocking)
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
                    continue;
                }

                // Check permission_behavior override (REQ-HOOK-004)
                if let Some(ref pb) = hook_agg.permission_behavior {
                    match pb {
                        crate::hooks::PermissionBehavior::Deny => {
                            let reason = hook_agg
                                .permission_decision_reason
                                .as_deref()
                                .unwrap_or("hook denied permission");
                            let result = ToolResult::error(format!("Permission denied: {reason}"));
                            self.send_event(AgentEvent::ToolCallComplete {
                                name: tool.name.clone(),
                                id: tool.id.clone(),
                                result: result.clone(),
                            })
                            .await;
                            self.state
                                .add_tool_result(&tool.id, &result.content, result.is_error);
                            continue;
                        }
                        crate::hooks::PermissionBehavior::Allow => {
                            // Skip normal permission check — hook allowed it
                            tracing::debug!(
                                tool = %tool.name,
                                "permission overridden to Allow by policy hook"
                            );
                        }
                        crate::hooks::PermissionBehavior::Ask => {
                            // TODO(Phase 2): force interactive prompt
                            tracing::debug!(
                                tool = %tool.name,
                                "permission_behavior=ask (not yet implemented, using normal flow)"
                            );
                        }
                        crate::hooks::PermissionBehavior::Passthrough => {
                            // No-op: normal permission flow proceeds
                        }
                    }
                }

                // Apply updated_input if hook modified it (REQ-HOOK-003)
                if let Some(modified_input) = hook_agg.updated_input {
                    if modified_input.is_object() {
                        tracing::debug!(
                            tool = %tool.name,
                            "PreToolUse hook modified tool input"
                        );
                        input = modified_input;
                    } else {
                        tracing::warn!(
                            tool = %tool.name,
                            "PreToolUse hook returned non-object updated_input, ignoring"
                        );
                    }
                }

                // Log system messages from hooks (REQ-HOOK-001)
                for msg in &hook_agg.system_messages {
                    tracing::warn!(tool = %tool.name, "[Hook Warning] {}", msg);
                }
                for msg in &hook_agg.status_messages {
                    tracing::info!(tool = %tool.name, "[Hook Status] {}", msg);
                }
            }

            // --- Resolve tool from registry ---
            let tool_arc = match self.registry.lookup(&tool.name) {
                Some(t) => t,
                None => {
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
                    continue;
                }
            };

            // --- Capture file_path for post-processing ---
            let file_path = if matches!(tool.name.as_str(), "Write" | "Edit" | "NotebookEdit") {
                input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };

            allowed.push(PreflightResult {
                tool_name: tool.name.clone(),
                tool_id: tool.id.clone(),
                input,
                tool_arc,
                file_path,
            });
        }

        allowed
    }
}
