use archon_llm::provider::LlmRequest;
use archon_llm::streaming::StreamEvent;
use tokio::sync::mpsc::Receiver;

use super::process_message_steps::PreparedTurnRequest;
use super::*;

fn request_pressure_recovery_exhausted() -> AgentLoopError {
    AgentLoopError::ApiError("request pressure recovery exhausted after two bounded retries".into())
}

impl Agent {
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

    pub(super) fn retry_request_with_messages(
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

    pub(super) fn emit_recovery_telemetry(
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
                return Err(request_pressure_recovery_exhausted());
            };
            let messages = match tier {
                autocompact::RecoveryTier::FullCompaction => {
                    match self.force_reactive_compact().await {
                        Ok(compacted) => tool_result_context::project_messages_for_request(
                            &compacted,
                            self.config.context.preserve_recent_turns,
                        ),
                        Err(AgentLoopError::Compaction(
                            autocompact::CompactionError::NoSafeBoundary,
                        )) => {
                            let Some(emergency) = recovery_ladder.next(classification) else {
                                return Err(request_pressure_recovery_exhausted());
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
}
