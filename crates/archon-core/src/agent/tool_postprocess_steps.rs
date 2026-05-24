use super::tool_types::PreflightResult;
use super::*;

#[derive(Default)]
pub(super) struct PostprocessFlow {
    pub(super) prevent_continuation_reason: Option<String>,
}

impl Agent {
    pub(super) async fn postprocess_single_tool(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
        ctx: &ToolContext,
        active_model: &str,
        flow: &mut PostprocessFlow,
    ) {
        let mut result = self.prepare_tool_result(pre, result, active_model).await;
        self.run_post_tool_hooks(pre, &mut result, ctx, flow).await;
        self.fire_path_hooks(pre).await;
        self.fire_worktree_hooks(pre).await;
        self.record_tool_completion(pre, &result).await;
        self.update_plan_progress(pre, &result).await;
        self.add_context_tool_result(pre, &result);
    }

    async fn prepare_tool_result(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
        active_model: &str,
    ) -> ToolResult {
        let result = self
            .maybe_handle_send_message_result(pre, result, active_model)
            .await;
        let result = self.handle_plan_mode_result(pre, result).await;
        self.handle_pending_user_input_result(result).await
    }

    async fn handle_plan_mode_result(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
    ) -> ToolResult {
        if result.is_error {
            return result;
        }
        match pre.tool_name.as_str() {
            "EnterPlanMode" => {
                let prev = self.config.permission_mode.lock().await.clone();
                self.previous_permission_mode = Some(prev);
                *self.config.permission_mode.lock().await = "plan".to_string();
                self.state.mode = AgentMode::Plan;
                result
            }
            "ExitPlanMode" => {
                self.restore_mode_after_plan().await;
                self.persist_latest_plan_from_assistant();
                result
            }
            _ => result,
        }
    }

    async fn restore_mode_after_plan(&mut self) {
        let restore = self
            .previous_permission_mode
            .take()
            .unwrap_or_else(|| "auto".to_string());
        *self.config.permission_mode.lock().await = restore;
        self.state.mode = AgentMode::Normal;
    }

    fn persist_latest_plan_from_assistant(&self) {
        let Some(ref plan_store) = self.plan_store else {
            return;
        };
        let plan_text = self.latest_assistant_text();
        if plan_text.is_empty() {
            return;
        }
        let plan = parse_plan_from_text(&plan_text);
        let sid = self.config.session_id.clone();
        match plan_store.save_plan(&sid, &plan) {
            Ok(()) => tracing::info!("plan saved: {} ({} steps)", plan.title, plan.steps.len()),
            Err(e) => tracing::warn!("failed to save plan: {e}"),
        }
    }

    fn latest_assistant_text(&self) -> String {
        self.state
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
            .unwrap_or_default()
    }

    async fn handle_pending_user_input_result(&mut self, result: ToolResult) -> ToolResult {
        if result.is_error || !result.content.starts_with("[PENDING_USER_INPUT]") {
            return result;
        }
        let question = result
            .content
            .strip_prefix("[PENDING_USER_INPUT]")
            .unwrap_or(&result.content)
            .to_string();
        let elicitation_agg = self
            .fire_hook(
                crate::hooks::HookEvent::Elicitation,
                serde_json::json!({
                    "hook_event": "Elicitation",
                    "question": question,
                }),
            )
            .await;
        if let Some(ref action) = elicitation_agg.elicitation_action {
            return self.auto_answer_elicitation(action, &elicitation_agg).await;
        }
        self.ask_user(question).await
    }

    async fn auto_answer_elicitation(
        &mut self,
        action: &crate::hooks::ElicitationAction,
        aggregate: &crate::hooks::AggregatedHookResult,
    ) -> ToolResult {
        let auto_response = match action {
            crate::hooks::ElicitationAction::Accept => aggregate
                .elicitation_content
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .unwrap_or_else(|_| Some("accepted".to_string()))
                .unwrap_or_else(|| "accepted".to_string()),
            crate::hooks::ElicitationAction::Decline => "declined".to_string(),
            crate::hooks::ElicitationAction::Cancel => "cancelled".to_string(),
        };
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
    }

    async fn ask_user(&mut self, question: String) -> ToolResult {
        self.send_event(AgentEvent::AskUser {
            question: question.clone(),
        })
        .await;
        if let Some(rx) = &self.ask_user_response_rx {
            match rx.lock().await.recv().await {
                Some(answer) => {
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
                None => ToolResult::error("User input channel closed unexpectedly.".to_string()),
            }
        } else {
            ToolResult::error(
                "User input requested but no input channel is configured.".to_string(),
            )
        }
    }

    async fn run_post_tool_hooks(
        &mut self,
        pre: &PreflightResult,
        result: &mut ToolResult,
        ctx: &ToolContext,
        flow: &mut PostprocessFlow,
    ) {
        let max_retries: u32 = 3;
        let mut retry_count: u32 = 0;
        loop {
            if result.is_error {
                self.fire_post_tool_failure_hook(pre, result).await;
                break;
            }
            let post_agg = self.fire_post_tool_success_hook(pre, result).await;
            self.apply_post_tool_aggregate(pre, result, &post_agg, flow);
            if post_agg.retry && retry_count < max_retries {
                retry_count += 1;
                tracing::info!(
                    tool = %pre.tool_name,
                    attempt = retry_count,
                    max = max_retries,
                    "PostToolUse hook requested retry, re-executing tool"
                );
                *result = pre.tool_arc.execute(pre.input.clone(), ctx).await;
                continue;
            }
            if post_agg.retry {
                tracing::warn!(
                    tool = %pre.tool_name,
                    "PostToolUse hook requested retry but max retries ({}) exceeded",
                    max_retries
                );
            }
            break;
        }
    }

    async fn fire_post_tool_failure_hook(&mut self, pre: &PreflightResult, result: &ToolResult) {
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
    }

    async fn fire_post_tool_success_hook(
        &mut self,
        pre: &PreflightResult,
        result: &ToolResult,
    ) -> crate::hooks::AggregatedHookResult {
        self.fire_hook(
            crate::hooks::HookEvent::PostToolUse,
            serde_json::json!({
                "hook_event": "PostToolUse",
                "tool_name": pre.tool_name,
                "tool_id": pre.tool_id,
                "result": result.content,
            }),
        )
        .await
    }

    fn apply_post_tool_aggregate(
        &self,
        pre: &PreflightResult,
        result: &mut ToolResult,
        post_agg: &crate::hooks::AggregatedHookResult,
        flow: &mut PostprocessFlow,
    ) {
        if let Some(modified_output) = post_agg.updated_mcp_tool_output.clone() {
            tracing::debug!(tool = %pre.tool_name, "PostToolUse hook modified tool output");
            let new_content = match modified_output {
                serde_json::Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_else(|_| other.to_string()),
            };
            *result = ToolResult::success(new_content);
        }
        if !post_agg.additional_contexts.is_empty() {
            let context = post_agg.additional_contexts.join("\n");
            *result = ToolResult::success(format!(
                "{}\n---\n[Hook Context]\n{}",
                result.content, context
            ));
        }
        for msg in &post_agg.system_messages {
            tracing::warn!(tool = %pre.tool_name, "[Hook Warning] {}", msg);
        }
        for msg in &post_agg.status_messages {
            tracing::info!(tool = %pre.tool_name, "[Hook Status] {}", msg);
        }
        if post_agg.prevent_continuation {
            let reason = post_agg
                .stop_reason
                .as_deref()
                .unwrap_or("hook requested stop");
            tracing::info!(
                tool = %pre.tool_name,
                "PostToolUse hook set prevent_continuation: {}", reason
            );
            flow.prevent_continuation_reason = Some(reason.to_string());
        }
    }

    async fn fire_path_hooks(&mut self, pre: &PreflightResult) {
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
            self.consume_watch_paths(file_agg.watch_paths);
        }
        if pre.tool_name == "Bash"
            && let Some(cmd) = pre.input.get("command").and_then(|v| v.as_str())
            && command_changes_cwd(cmd)
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
            self.consume_watch_paths(cwd_agg.watch_paths);
        }
    }

    fn consume_watch_paths(&self, watch_paths: Vec<String>) {
        if watch_paths.is_empty() {
            return;
        }
        tracing::info!("Hook returned {} watch paths", watch_paths.len());
        self.file_watch_manager.add_watch_paths(watch_paths);
    }

    async fn fire_worktree_hooks(&mut self, pre: &PreflightResult) {
        let hook = match pre.tool_name.as_str() {
            "EnterWorktree" => crate::hooks::HookEvent::WorktreeCreate,
            "ExitWorktree" => crate::hooks::HookEvent::WorktreeRemove,
            _ => return,
        };
        self.fire_hook(
            hook,
            serde_json::json!({
                "hook_event": match pre.tool_name.as_str() {
                    "EnterWorktree" => "WorktreeCreate",
                    _ => "WorktreeRemove",
                },
                "tool_name": pre.tool_name,
            }),
        )
        .await;
    }

    async fn record_tool_completion(&mut self, pre: &PreflightResult, result: &ToolResult) {
        self.fire_after_tool_call_hook(&pre.tool_name, &pre.tool_id, result)
            .await;
        self.record_reasoning_tool_evidence(
            &pre.tool_name,
            &pre.tool_id,
            &pre.input,
            result,
            pre.file_path.as_deref(),
        );
        self.send_event(AgentEvent::ToolCallComplete {
            name: pre.tool_name.clone(),
            id: pre.tool_id.clone(),
            result: result.clone(),
        })
        .await;
        self.update_inner_voice_for_tool(&pre.tool_name, result)
            .await;
    }

    async fn update_inner_voice_for_tool(&mut self, tool_name: &str, result: &ToolResult) {
        if let Some(iv) = &self.inner_voice {
            let mut iv = iv.lock().await;
            if result.is_error {
                iv.on_tool_failure(tool_name);
            } else {
                iv.on_tool_success(tool_name);
            }
            if let Some(ref cb) = self.inner_voice_change_callback {
                cb(&iv);
            }
        }
    }

    async fn update_plan_progress(&mut self, pre: &PreflightResult, result: &ToolResult) {
        if result.is_error || (pre.tool_name != "Write" && pre.tool_name != "Edit") {
            return;
        }
        let Some(ref plan_store) = self.plan_store else {
            return;
        };
        let Some(ref fp) = pre.file_path else {
            return;
        };
        let sid = self.config.session_id.clone();
        if let Ok(Some(plan)) = plan_store.load_latest_plan(&sid)
            && (plan.status == "active" || plan.status == "draft")
        {
            for step in &plan.steps {
                if plan_step_matches_file(step, fp)
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

    fn add_context_tool_result(&mut self, pre: &PreflightResult, result: &ToolResult) {
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
}

fn command_changes_cwd(cmd: &str) -> bool {
    let trimmed = cmd.trim_start();
    trimmed.starts_with("cd ") || cmd.contains(" && cd ") || cmd.contains("; cd ")
}

fn plan_step_matches_file(step: &archon_session::plan::PlanStep, file_path: &str) -> bool {
    step.status == archon_session::plan::PlanStepStatus::Pending
        && step
            .affected_files
            .iter()
            .any(|f| file_path.ends_with(f) || f.ends_with(file_path))
}
