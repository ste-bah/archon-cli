use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn postprocess_tools(
        &mut self,
        allowed: &[PreflightResult],
        dispatch_results: Vec<ToolResult>,
        ctx: &ToolContext,
        active_model: &str,
    ) -> Option<String> {
        // -------------------------------------------------------
        // PHASE 3: Post-process (sequential)
        // Handle interceptions, fire post-hooks, emit events,
        // update inner voice, record results in conversation state.
        // -------------------------------------------------------
        let mut prevent_continuation_reason: Option<String> = None;
        for (pre, result) in allowed.iter().zip(dispatch_results.into_iter()) {
            // TASK-AGS-105: AgentTool / TaskCreate now return their
            // final user-facing ToolResult directly via the
            // SubagentExecutor seam. No re-parse or indirection here.

            let result = self
                .maybe_handle_send_message_result(pre, result, active_model)
                .await;

            // CRIT-08: Intercept EnterPlanMode / ExitPlanMode.
            let result = if !result.is_error && pre.tool_name == "EnterPlanMode" {
                let prev = self.config.permission_mode.lock().await.clone();
                self.previous_permission_mode = Some(prev);
                *self.config.permission_mode.lock().await = "plan".to_string();
                self.state.mode = AgentMode::Plan;
                result
            } else if !result.is_error && pre.tool_name == "ExitPlanMode" {
                let restore = self
                    .previous_permission_mode
                    .take()
                    .unwrap_or_else(|| "auto".to_string());
                *self.config.permission_mode.lock().await = restore;
                self.state.mode = AgentMode::Normal;

                // Wire 2: Parse plan from assistant text and persist.
                if let Some(ref plan_store) = self.plan_store {
                    // Get the last assistant message's text content
                    let plan_text = self
                        .state
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m["role"].as_str() == Some("assistant"))
                        .and_then(|m| match &m["content"] {
                            serde_json::Value::Array(blocks) => blocks
                                .iter()
                                .find(|b| b["type"].as_str() == Some("text"))
                                .and_then(|b| b["text"].as_str())
                                .map(|s| s.to_string()),
                            serde_json::Value::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    if !plan_text.is_empty() {
                        let plan = parse_plan_from_text(&plan_text);
                        let sid = self.config.session_id.clone();
                        match plan_store.save_plan(&sid, &plan) {
                            Ok(()) => tracing::info!(
                                "plan saved: {} ({} steps)",
                                plan.title,
                                plan.steps.len()
                            ),
                            Err(e) => tracing::warn!("failed to save plan: {e}"),
                        }
                    }
                }

                result
            } else {
                result
            };

            // CRIT-09: Intercept AskUserQuestion sentinel.
            let mut result = if !result.is_error
                && result.content.starts_with("[PENDING_USER_INPUT]")
            {
                let question = result
                    .content
                    .strip_prefix("[PENDING_USER_INPUT]")
                    .unwrap_or(&result.content)
                    .to_string();

                // CRIT-06: Fire Elicitation hook before presenting question to user
                let elicitation_agg = self
                    .fire_hook(
                        crate::hooks::HookEvent::Elicitation,
                        serde_json::json!({
                            "hook_event": "Elicitation",
                            "question": question,
                        }),
                    )
                    .await;

                // REQ-HOOK-019: If hook returns elicitation_action, auto-respond
                if let Some(ref action) = elicitation_agg.elicitation_action {
                    let auto_response = match action {
                        crate::hooks::ElicitationAction::Accept => {
                            if let Some(ref content) = elicitation_agg.elicitation_content {
                                serde_json::to_string(content)
                                    .unwrap_or_else(|_| "accepted".to_string())
                            } else {
                                "accepted".to_string()
                            }
                        }
                        crate::hooks::ElicitationAction::Decline => "declined".to_string(),
                        crate::hooks::ElicitationAction::Cancel => "cancelled".to_string(),
                    };

                    // Fire ElicitationResult with auto-response
                    self.fire_hook(
                        crate::hooks::HookEvent::ElicitationResult,
                        serde_json::json!({
                            "hook_event": "ElicitationResult",
                            "result": &auto_response,
                            "auto_responded": true,
                        }),
                    )
                    .await;

                    ToolResult::success(auto_response)
                } else {
                    self.send_event(AgentEvent::AskUser {
                        question: question.clone(),
                    })
                    .await;

                    if let Some(rx) = &self.ask_user_response_rx {
                        match rx.lock().await.recv().await {
                            Some(answer) => {
                                // CRIT-06: Fire ElicitationResult hook after user responds
                                self.fire_hook(
                                    crate::hooks::HookEvent::ElicitationResult,
                                    serde_json::json!({
                                        "hook_event": "ElicitationResult",
                                        "result": &answer,
                                    }),
                                )
                                .await;
                                ToolResult::success(answer)
                            }
                            None => ToolResult::error(
                                "User input channel closed unexpectedly.".to_string(),
                            ),
                        }
                    } else {
                        ToolResult::error(
                            "User input requested but no input channel is configured.".to_string(),
                        )
                    }
                } // end else (no elicitation_action)
            } else {
                result
            };

            // CRIT-06: Fire PostToolUse / PostToolUseFailure hooks (REQ-HOOK-005)
            // Retry loop: max 3 re-executions if PostToolUse hook sets retry=true
            let max_retries: u32 = 3;
            let mut retry_count: u32 = 0;
            loop {
                if result.is_error {
                    let _post_agg = self
                        .fire_hook(
                            crate::hooks::HookEvent::PostToolUseFailure,
                            serde_json::json!({
                                "hook_event": "PostToolUseFailure",
                                "tool_name": pre.tool_name,
                                "tool_id": pre.tool_id,
                                "error": result.content,
                            }),
                        )
                        .await;
                    break; // No retry on failure
                }

                let post_agg = self
                    .fire_hook(
                        crate::hooks::HookEvent::PostToolUse,
                        serde_json::json!({
                            "hook_event": "PostToolUse",
                            "tool_name": pre.tool_name,
                            "tool_id": pre.tool_id,
                            "result": result.content,
                        }),
                    )
                    .await;

                // Apply updated_mcp_tool_output (REQ-HOOK-005)
                if let Some(modified_output) = post_agg.updated_mcp_tool_output {
                    tracing::debug!(
                        tool = %pre.tool_name,
                        "PostToolUse hook modified tool output"
                    );
                    let new_content = match modified_output {
                        serde_json::Value::String(s) => s,
                        other => {
                            serde_json::to_string(&other).unwrap_or_else(|_| other.to_string())
                        }
                    };
                    result = ToolResult::success(new_content);
                }

                // Append additional_contexts (REQ-HOOK-005)
                if !post_agg.additional_contexts.is_empty() {
                    let context = post_agg.additional_contexts.join("\n");
                    result = ToolResult::success(format!(
                        "{}\n---\n[Hook Context]\n{}",
                        result.content, context
                    ));
                }

                // Log system/status messages from PostToolUse hooks
                for msg in &post_agg.system_messages {
                    tracing::warn!(tool = %pre.tool_name, "[Hook Warning] {}", msg);
                }
                for msg in &post_agg.status_messages {
                    tracing::info!(tool = %pre.tool_name, "[Hook Status] {}", msg);
                }

                // Handle prevent_continuation (REQ-HOOK-005 flow control)
                if post_agg.prevent_continuation {
                    let reason = post_agg
                        .stop_reason
                        .as_deref()
                        .unwrap_or("hook requested stop");
                    tracing::info!(
                        tool = %pre.tool_name,
                        "PostToolUse hook set prevent_continuation: {}", reason
                    );
                    prevent_continuation_reason = Some(reason.to_string());
                }

                // Handle retry (REQ-HOOK-005 flow control)
                if post_agg.retry && retry_count < max_retries {
                    retry_count += 1;
                    tracing::info!(
                        tool = %pre.tool_name,
                        attempt = retry_count,
                        max = max_retries,
                        "PostToolUse hook requested retry, re-executing tool"
                    );
                    result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                    continue; // Loop back to fire PostToolUse again
                } else if post_agg.retry {
                    tracing::warn!(
                        tool = %pre.tool_name,
                        "PostToolUse hook requested retry but max retries ({}) exceeded",
                        max_retries
                    );
                }

                break; // Normal exit — no retry requested or retries exhausted
            }

            if let Some(ref fp) = pre.file_path {
                let file_agg = self
                    .fire_hook(
                        crate::hooks::HookEvent::FileChanged,
                        serde_json::json!({
                            "hook_event": "FileChanged",
                            "tool_name": pre.tool_name,
                            "file_path": fp,
                        }),
                    )
                    .await;
                // Consume watch_paths from FileChanged hooks (REQ-HOOK-017)
                if !file_agg.watch_paths.is_empty() {
                    tracing::info!("Hook returned {} watch paths", file_agg.watch_paths.len());
                    self.file_watch_manager
                        .add_watch_paths(file_agg.watch_paths);
                }
            }

            // CRIT-06: Fire CwdChanged if a Bash tool call changed the working directory
            if pre.tool_name == "Bash"
                && let Some(cmd) = pre.input.get("command").and_then(|v| v.as_str())
                && (cmd.trim_start().starts_with("cd ")
                    || cmd.contains(" && cd ")
                    || cmd.contains("; cd "))
            {
                let cwd_agg = self
                    .fire_hook(
                        crate::hooks::HookEvent::CwdChanged,
                        serde_json::json!({
                            "hook_event": "CwdChanged",
                            "command": cmd,
                        }),
                    )
                    .await;
                // Consume watch_paths from CwdChanged hooks (REQ-HOOK-017)
                if !cwd_agg.watch_paths.is_empty() {
                    tracing::info!("Hook returned {} watch paths", cwd_agg.watch_paths.len());
                    self.file_watch_manager.add_watch_paths(cwd_agg.watch_paths);
                }
            }

            // CRIT-06: Fire WorktreeCreate/WorktreeRemove based on tool name
            if pre.tool_name == "EnterWorktree" {
                self.fire_hook(
                    crate::hooks::HookEvent::WorktreeCreate,
                    serde_json::json!({
                        "hook_event": "WorktreeCreate",
                        "tool_name": pre.tool_name,
                    }),
                )
                .await;
            } else if pre.tool_name == "ExitWorktree" {
                self.fire_hook(
                    crate::hooks::HookEvent::WorktreeRemove,
                    serde_json::json!({
                        "hook_event": "WorktreeRemove",
                        "tool_name": pre.tool_name,
                    }),
                )
                .await;
            }

            self.fire_after_tool_call_hook(&pre.tool_name, &pre.tool_id, &result)
                .await;
            self.record_reasoning_tool_evidence(
                &pre.tool_name,
                &pre.tool_id,
                &pre.input,
                &result,
                pre.file_path.as_deref(),
            );

            self.send_event(AgentEvent::ToolCallComplete {
                name: pre.tool_name.clone(),
                id: pre.tool_id.clone(),
                result: result.clone(),
            })
            .await;

            if let Some(iv) = &self.inner_voice {
                let mut iv = iv.lock().await;
                if result.is_error {
                    iv.on_tool_failure(&pre.tool_name);
                } else {
                    iv.on_tool_success(&pre.tool_name);
                }
                // TASK #245: keep panic-mirror in lock-step.
                if let Some(ref cb) = self.inner_voice_change_callback {
                    cb(&iv);
                }
            }

            // Wire 3: Track plan step progress on Write/Edit completions.
            if !result.is_error
                && (pre.tool_name == "Write" || pre.tool_name == "Edit")
                && let Some(ref plan_store) = self.plan_store
            {
                let sid = self.config.session_id.clone();
                if let Ok(Some(plan)) = plan_store.load_latest_plan(&sid)
                    && (plan.status == "active" || plan.status == "draft")
                    && let Some(ref fp) = pre.file_path
                {
                    for step in &plan.steps {
                        if step.status == archon_session::plan::PlanStepStatus::Pending
                            && step
                                .affected_files
                                .iter()
                                .any(|f| fp.ends_with(f) || f.ends_with(fp))
                            && let Err(e) = plan_store.update_step_status(
                                &sid,
                                &plan.id,
                                step.number,
                                archon_session::plan::PlanStepStatus::InProgress,
                            )
                        {
                            tracing::debug!("plan step update failed: {e}");
                        }
                    }
                }
            }

            let context_output = crate::agent::tool_result_context::cap_tool_output_for_context(
                &pre.tool_name,
                &result.content,
            );
            if context_output.truncated {
                tracing::warn!(
                    tool = %pre.tool_name,
                    tool_use_id = %pre.tool_id,
                    original_chars = context_output.original_chars,
                    stored_chars = context_output.stored_chars,
                    limit_chars = context_output.limit_chars,
                    "tool output trimmed before model replay"
                );
            }
            self.state
                .add_tool_result(&pre.tool_id, &context_output.content, result.is_error);
        }

        // Atomicity safety net (v1.2.6): after dispatch + postprocess, every
        // tool_use block in the last assistant message MUST have a matching
        // tool_result in the immediately following user message. If
        // `dispatch_results.len() < allowed.len()` (panic mid-dispatch,
        // cancellation, length mismatch, etc.) some tool_uses never get a
        // tool_result, leaving the persisted state.messages in a shape that
        // Anthropic rejects on the next API call with `tool_use ids were
        // found without tool_result blocks immediately after`. Fill the gap
        // here with synthetic error tool_results so state.messages always
        // round-trips through the Anthropic API.
        self.fill_orphan_tool_results(allowed);

        prevent_continuation_reason
    }

    fn fill_orphan_tool_results(&mut self, allowed: &[PreflightResult]) {
        let expected_ids: Vec<String> = allowed.iter().map(|p| p.tool_id.clone()).collect();
        for id in self.state.fill_missing_tool_results(&expected_ids) {
            tracing::warn!(
                tool_use_id = %id,
                "tool dispatch did not produce a result; filled with synthetic error"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_orphan_tool_results_appends_synthetic_for_missing_ids() {
        let mut state = ConversationState::default();
        state.add_assistant_message(vec![
            serde_json::json!({"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}}),
            serde_json::json!({"type": "tool_use", "id": "tool-2", "name": "Write", "input": {}}),
        ]);
        state.add_tool_result("tool-1", "ok", false);

        let missing =
            state.fill_missing_tool_results(&["tool-1".to_string(), "tool-2".to_string()]);

        assert_eq!(missing, vec!["tool-2".to_string()]);
        let blocks = state.messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["tool_use_id"], "tool-2");
        assert_eq!(blocks[1]["is_error"], true);
    }

    #[test]
    fn fill_orphan_tool_results_idempotent_when_all_present() {
        let mut state = ConversationState::default();
        state.add_tool_result("tool-1", "ok", false);

        let missing = state.fill_missing_tool_results(&["tool-1".to_string()]);

        assert!(missing.is_empty());
        assert_eq!(state.messages[0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn fill_orphan_tool_results_no_op_when_allowed_empty() {
        let mut state = ConversationState::default();

        let missing = state.fill_missing_tool_results(&[]);

        assert!(missing.is_empty());
        assert!(state.messages.is_empty());
    }

    #[test]
    fn context_tool_output_cap_preserves_ui_result_shape() {
        let content = "x".repeat(80_000);
        let capped =
            crate::agent::tool_result_context::cap_tool_output_for_context("Agent", &content);

        assert!(capped.truncated);
        assert!(capped.content.contains("tool output trimmed"));
        assert!(capped.stored_chars <= capped.limit_chars);
    }
}
