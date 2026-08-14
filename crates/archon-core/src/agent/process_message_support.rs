use archon_llm::effort::EffortLevel;
use std::sync::Arc;

use super::process_message_steps::{PreparedTurnRequest, StreamRoundOutcome, ToolLoopAction};
use super::*;

impl Agent {
    pub(super) fn spawn_auto_extraction(&mut self) {
        let Some(ref extractor) = self.auto_extractor else {
            return;
        };
        let extractor = Arc::clone(extractor);
        let turns = self.conversation_text_turns();
        let model = self.config.model.clone();
        let turn = self.turn_number as u32;
        let attribution = self.config.runtime_attribution_extra(
            "memory_extraction",
            "auto_extraction",
            Some(self.turn_number),
            None,
            None,
        );
        self.prune_finished_auto_extractions();
        let handle = tokio::spawn(async move {
            let _ = extractor
                .maybe_extract(&turns, turn, &model, attribution)
                .await;
        });
        self.auto_extraction_tasks.push(handle);
    }
    pub(super) async fn active_model(&self) -> String {
        let override_model = self.config.model_override.lock().await;
        if override_model.is_empty() {
            self.config.model.clone()
        } else {
            override_model.clone()
        }
    }
    /// The effort level for this turn, always as a concrete level (#123).
    ///
    /// This used to return `None` for `High`, and `None` for any turn
    /// containing `ultrathink`. That encoded an Anthropic convention —
    /// omitting `output_config.effort` means high there — inside a
    /// provider-agnostic layer. On an OpenAI-compatible backend an absent
    /// `reasoning_effort` means "no reasoning at all", so the omission
    /// silently disabled reasoning on vLLM instead of maximising it. Each
    /// provider now clamps the level itself: `effective_effort` for Anthropic,
    /// `clamp_reasoning_effort` for Codex, the configured `effort_map` for
    /// OpenAI-compatible backends.
    ///
    /// `ultrathink` raises the level to `Max` for this turn only. It uses
    /// `raised_to`, so an explicitly higher level is never lowered, and it
    /// does not touch the persisted level — the user's `/effort medium` is
    /// still in force next turn.
    pub(super) async fn turn_effort(&self, user_input: &str) -> Option<String> {
        let level = *self.config.effort_level.lock().await;
        let level = if archon_llm::thinking::ultrathink_requested(user_input) {
            level.raised_to(EffortLevel::Max)
        } else {
            level
        };
        Some(level.to_string())
    }
    pub(super) async fn fail_parent_turn(&mut self, message: String) {
        self.emit_activity(
            AgentActivityKind::ParentTurnCompleted,
            AgentActivityStatus::Failed,
            format!("turn {} failed: {message}", self.turn_number),
        );
        self.fire_after_agent_run_hook("failed", Some(message))
            .await;
    }

    pub(super) async fn record_pending_tool_start(
        &mut self,
        pending_tools: &mut Vec<PendingToolCall>,
        pending_tool_indices: &mut Vec<u32>,
        index: u32,
        tool_use_id: Option<String>,
        tool_name: Option<String>,
    ) {
        let id = tool_use_id.unwrap_or_default();
        let name = tool_name.unwrap_or_default();
        self.send_event(AgentEvent::ToolCallStarted {
            name: name.clone(),
            id: id.clone(),
        })
        .await;
        pending_tools.push(PendingToolCall {
            id,
            name,
            input_json: String::new(),
        });
        pending_tool_indices.push(index);
    }

    pub(super) async fn handle_stream_error(
        &mut self,
        error_type: String,
        message: String,
        prepared: &PreparedTurnRequest,
        recovery_ladder: &mut autocompact::RecoveryLadder,
        reactive_rate_limit_retried: &mut bool,
    ) -> Result<StreamRoundOutcome, AgentLoopError> {
        let classified =
            autocompact::classify_stream_error(self.client.name(), &error_type, &message);
        if let Some(classification) =
            autocompact::request_pressure_kind_for_request(&classified, &prepared.request)
        {
            self.discard_thinking_preview().await;
            let tier = recovery_ladder.next(classification);
            let Some(tier) = tier else {
                return Err(AgentLoopError::ApiError(
                    "request pressure recovery exhausted after two bounded retries".into(),
                ));
            };
            return match tier {
                autocompact::RecoveryTier::FullCompaction => {
                    match self.force_reactive_compact().await {
                        Ok(compacted) => {
                            let messages = tool_result_context::project_messages_for_request(
                                &compacted,
                                self.config.context.preserve_recent_turns,
                            );
                            let retry_request =
                                self.retry_request_with_messages(prepared, messages);
                            self.emit_recovery_telemetry(
                                classification,
                                tier,
                                prepared.request_body_bytes,
                                &retry_request,
                            );
                            match self.client.stream(retry_request.clone()).await {
                                Ok(rx) => Ok(StreamRoundOutcome::RetryStream(rx)),
                                Err(error) => {
                                    let Some(next_classification) =
                                        autocompact::request_pressure_kind_for_request(
                                            &error,
                                            &retry_request,
                                        )
                                    else {
                                        return Err(AgentLoopError::ApiError(format!(
                                            "full compaction retry failed: {error}"
                                        )));
                                    };
                                    self.recover_request_pressure(
                                        prepared,
                                        recovery_ladder,
                                        next_classification,
                                    )
                                    .await
                                    .map(StreamRoundOutcome::RetryStream)
                                }
                            }
                        }
                        Err(AgentLoopError::Compaction(
                            autocompact::CompactionError::NoSafeBoundary,
                        )) => {
                            let Some(emergency) = recovery_ladder.next(classification) else {
                                return Err(AgentLoopError::ApiError(
                                    "request pressure recovery exhausted after two bounded retries"
                                        .into(),
                                ));
                            };
                            debug_assert_eq!(
                                emergency,
                                autocompact::RecoveryTier::EmergencyProjection
                            );
                            let messages =
                                tool_result_context::project_messages_for_emergency_retry(
                                    &self.state.messages,
                                    tool_result_context::emergency_tool_result_bytes(
                                        self.state.max_tool_result_bytes,
                                    ),
                                );
                            let retry_request =
                                self.retry_request_with_messages(prepared, messages);
                            self.emit_recovery_telemetry(
                                classification,
                                emergency,
                                prepared.request_body_bytes,
                                &retry_request,
                            );
                            self.client
                                .stream(retry_request)
                                .await
                                .map_err(|retry| {
                                    AgentLoopError::ApiError(format!(
                                        "emergency projection retry failed: {retry}"
                                    ))
                                })
                                .map(StreamRoundOutcome::RetryStream)
                        }
                        Err(error) => Err(error),
                    }
                }
                autocompact::RecoveryTier::EmergencyProjection => {
                    let messages = tool_result_context::project_messages_for_emergency_retry(
                        &self.state.messages,
                        tool_result_context::emergency_tool_result_bytes(
                            self.state.max_tool_result_bytes,
                        ),
                    );
                    let retry_request = self.retry_request_with_messages(prepared, messages);
                    self.emit_recovery_telemetry(
                        classification,
                        tier,
                        prepared.request_body_bytes,
                        &retry_request,
                    );
                    self.client
                        .stream(retry_request)
                        .await
                        .map_err(|retry| {
                            AgentLoopError::ApiError(format!(
                                "emergency projection retry failed: {retry}"
                            ))
                        })
                        .map(StreamRoundOutcome::RetryStream)
                }
            };
        }
        if autocompact::is_rate_limited_error(&classified)
            && !*reactive_rate_limit_retried
            && prepared.request_body_bytes >= prepared.large_retry_body_bytes
        {
            self.discard_thinking_preview().await;
            *reactive_rate_limit_retried = true;
            self.warn_large_rate_limit(prepared, "rate_limit_large_request_stream");
            let compacted = self.force_reactive_compact().await?;
            let messages = tool_result_context::project_messages_for_request(
                &compacted,
                self.config.context.preserve_recent_turns,
            );
            let retry_request = self.retry_request_with_messages(prepared, messages);
            self.emit_recovery_telemetry(
                autocompact::RequestPressureKind::AggregateContext,
                autocompact::RecoveryTier::FullCompaction,
                prepared.request_body_bytes,
                &retry_request,
            );
            return self
                .client
                .stream(retry_request)
                .await
                .map_err(|retry| {
                    AgentLoopError::ApiError(format!("rate-limit compaction retry failed: {retry}"))
                })
                .map(StreamRoundOutcome::RetryStream);
        }
        self.fire_hook(
            crate::hooks::HookEvent::Notification,
            serde_json::json!({
                "hook_event": "Notification",
                "level": "error",
                "message": format!("{error_type}: {message}"),
            }),
        )
        .await;
        self.send_event(AgentEvent::DiscardThinkingPreview).await;
        self.send_event(AgentEvent::Error(format!("{error_type}: {message}")))
            .await;
        self.fail_parent_turn(format!("{error_type}: {message}"))
            .await;
        Err(AgentLoopError::ApiError(format!("{error_type}: {message}")))
    }

    pub(super) fn assistant_tool_use_block(&self, tool: &PendingToolCall) -> serde_json::Value {
        let allow_empty = self
            .registry
            .lookup(&tool.name)
            .map(|tool_arc| tool_input_json::schema_allows_empty_input(&tool_arc.input_schema()))
            .unwrap_or(false);
        let input = match tool_input_json::parse_pending_tool_input(
            &tool.name,
            &tool.id,
            &tool.input_json,
            allow_empty,
        ) {
            Ok(input) => input,
            Err(err) => malformed_tool_input(tool, err),
        };
        serde_json::json!({
            "type": "tool_use",
            "id": tool.id,
            "name": tool.name,
            "input": input,
        })
    }

    pub(super) async fn handle_pending_tool_round(
        &mut self,
        pending_tools: &[PendingToolCall],
        active_model: &str,
        agentic_iterations: &mut u32,
    ) -> ToolLoopAction {
        let effective_mode = self.effective_agent_mode().await;
        let ctx = self.build_tool_context(effective_mode, active_model).await;
        let allowed = self.preflight_tools(pending_tools, effective_mode).await;
        let dispatch_results = self.dispatch_allowed_tools(&allowed, &ctx).await;
        if let Some(reason) = self
            .postprocess_tools(&allowed, dispatch_results, &ctx, active_model)
            .await
        {
            tracing::info!("Hook requested conversation stop: {}", reason);
            return ToolLoopAction::Break;
        }
        *agentic_iterations += 1;
        self.check_agentic_turn_limit(*agentic_iterations).await
    }

    pub(super) async fn effective_agent_mode(&self) -> AgentMode {
        let pm = self.config.permission_mode.lock().await;
        if pm.as_str() == "plan" {
            AgentMode::Plan
        } else {
            AgentMode::Normal
        }
    }

    async fn check_agentic_turn_limit(&mut self, agentic_iterations: u32) -> ToolLoopAction {
        let Some(max) = self.config.max_turns else {
            return ToolLoopAction::Continue;
        };
        if agentic_iterations < max {
            return ToolLoopAction::Continue;
        }
        tracing::info!(
            "max_turns limit reached ({}/{}), stopping agentic loop",
            agentic_iterations,
            max
        );
        self.send_event(AgentEvent::Error(format!(
            "Agentic turn limit reached ({max} turns). Stopping."
        )))
        .await;
        ToolLoopAction::Break
    }

    fn conversation_text_turns(&self) -> Vec<String> {
        self.state
            .messages
            .iter()
            .filter_map(message_text_content)
            .collect()
    }
}

pub(super) fn append_tool_input_delta(
    pending_tools: &mut [PendingToolCall],
    pending_tool_indices: &[u32],
    index: u32,
    partial_json: &str,
) {
    if !tool_input_json::append_delta_by_index(
        pending_tools,
        pending_tool_indices,
        index,
        partial_json,
        |tool, delta| tool.input_json.push_str(delta),
    ) {
        tracing::warn!(
            tool_block_index = index,
            "received tool input JSON delta without matching tool block"
        );
    }
}

fn malformed_tool_input(tool: &PendingToolCall, err: String) -> serde_json::Value {
    tracing::warn!(
        tool = %tool.name,
        tool_use_id = %tool.id,
        input_len = tool.input_json.len(),
        "{err}"
    );
    serde_json::json!({
        "_archon_malformed_tool_input": true,
        "error": err,
    })
}
