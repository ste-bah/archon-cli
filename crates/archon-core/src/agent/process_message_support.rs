use std::sync::Arc;

use archon_llm::effort::EffortLevel;
use archon_llm::provider::LlmRequest;
use archon_llm::streaming::StreamEvent;
use tokio::sync::mpsc::Receiver;

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

    pub(super) async fn turn_effort(&self, user_input: &str) -> Option<String> {
        if user_input.to_lowercase().contains("ultrathink") {
            return None;
        }
        let level = self.config.effort_level.lock().await;
        match *level {
            EffortLevel::High => None,
            other => Some(other.to_string()),
        }
    }

    pub(super) async fn retry_stream_with_messages(
        &self,
        prepared: &PreparedTurnRequest,
        messages: Vec<serde_json::Value>,
        retry_label: &str,
    ) -> Result<Receiver<StreamEvent>, AgentLoopError> {
        let retry_request = self.retry_request_with_messages(prepared, messages);
        self.client
            .stream(retry_request)
            .await
            .map_err(|retry| AgentLoopError::ApiError(format!("{retry_label}: {retry}")))
    }

    fn retry_request_with_messages(
        &self,
        prepared: &PreparedTurnRequest,
        messages: Vec<serde_json::Value>,
    ) -> LlmRequest {
        let mut retry_request = LlmRequest {
            messages,
            ..prepared.request.clone()
        };
        request_cache::apply_conversation_cache(
            &mut retry_request,
            self.client.as_ref(),
            self.config.context.prompt_cache && self.config.context.prompt_cache_conversation,
            &self.config.context.prompt_cache_mode,
            &self.config.context.prompt_cache_ttl,
        );
        retry_request
    }

    fn emit_recovery_telemetry(
        &self,
        classification: autocompact::RequestPressureKind,
        tier: autocompact::RecoveryTier,
        before_body_bytes: usize,
        retry_request: &LlmRequest,
    ) {
        let telemetry = autocompact::RecoveryTelemetry::new(
            classification,
            tier,
            before_body_bytes,
            autocompact::request_body_bytes(retry_request),
        )
        .with_cooldown_secs(self.state.auto_compact.cooldown_remaining_secs());
        self.emit_activity(
            AgentActivityKind::AgentRunning,
            AgentActivityStatus::Running,
            serde_json::to_string(&telemetry)
                .unwrap_or_else(|_| "request pressure recovery".into()),
        );
        tracing::warn!(
            compaction.classification = ?telemetry.classification,
            compaction.tier = ?telemetry.tier,
            before_body_bytes = telemetry.before_body_bytes,
            after_body_bytes = telemetry.after_body_bytes,
            before_estimated_tokens = telemetry.before_estimated_tokens,
            after_estimated_tokens = telemetry.after_estimated_tokens,
            reduced = telemetry.reduced,
            scope = "main_session",
            "request pressure recovery retry"
        );
    }

    pub(super) fn warn_large_rate_limit(&self, prepared: &PreparedTurnRequest, reason: &str) {
        let telemetry = self.compaction_telemetry_for(&prepared.active_model);
        tracing::warn!(
            compaction.reason = reason,
            trigger_body_bytes = prepared.request_body_bytes,
            threshold_body_bytes = prepared.large_retry_body_bytes,
            provider_family = telemetry.provider_family,
            wire_shape = telemetry.wire_shape,
            native_context_window = telemetry.native_context_window,
            runtime_context_budget = telemetry.runtime_context_budget,
            context_source = telemetry.context_source,
            compaction_backend = telemetry.compaction_backend,
            scope = "main_session",
            force = true,
            "rate-limited main request is large; compacting before one retry"
        );
    }

    pub(super) async fn recover_request_pressure(
        &mut self,
        prepared: &PreparedTurnRequest,
        recovery_ladder: &mut autocompact::RecoveryLadder,
        mut classification: autocompact::RequestPressureKind,
    ) -> Result<Receiver<StreamEvent>, AgentLoopError> {
        let mut before_body_bytes = prepared.request_body_bytes;
        loop {
            let Some(mut tier) = recovery_ladder.next(classification) else {
                return Err(AgentLoopError::ApiError(
                    "request pressure recovery exhausted after two bounded retries".into(),
                ));
            };
            let messages = match tier {
                autocompact::RecoveryTier::FullCompaction => {
                    match self.force_reactive_compact().await {
                        Ok(()) => tool_result_context::project_messages_for_request(
                            &self.state.messages,
                            self.config.context.preserve_recent_turns,
                        ),
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
                            tier = emergency;
                            tool_result_context::project_messages_for_emergency_retry(
                                &self.state.messages,
                                tool_result_context::emergency_tool_result_bytes(
                                    self.state.max_tool_result_bytes,
                                ),
                            )
                        }
                        Err(error) => return Err(error),
                    }
                }
                autocompact::RecoveryTier::EmergencyProjection => {
                    tool_result_context::project_messages_for_emergency_retry(
                        &self.state.messages,
                        tool_result_context::emergency_tool_result_bytes(
                            self.state.max_tool_result_bytes,
                        ),
                    )
                }
            };
            let retry_request = self.retry_request_with_messages(prepared, messages);
            self.emit_recovery_telemetry(classification, tier, before_body_bytes, &retry_request);
            match self.client.stream(retry_request.clone()).await {
                Ok(rx) => return Ok(rx),
                Err(error) => {
                    let Some(next_classification) =
                        autocompact::request_pressure_kind_for_request(&error, &retry_request)
                    else {
                        return Err(AgentLoopError::ApiError(format!(
                            "request pressure retry failed: {error}"
                        )));
                    };
                    classification = next_classification;
                    before_body_bytes = autocompact::request_body_bytes(&retry_request);
                }
            }
        }
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
                        Ok(()) => {
                            let messages = tool_result_context::project_messages_for_request(
                                &self.state.messages,
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
                            Ok(StreamRoundOutcome::RetryAgentLoop)
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
            self.force_reactive_compact().await?;
            return Ok(StreamRoundOutcome::RetryAgentLoop);
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

    async fn effective_agent_mode(&self) -> AgentMode {
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
