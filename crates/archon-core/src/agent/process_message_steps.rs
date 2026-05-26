use archon_llm::provider::LlmRequest;
use archon_llm::streaming::StreamEvent;
use archon_llm::usage::UsageAccumulator;
use tokio::sync::mpsc::Receiver;

use super::process_message_support::append_tool_input_delta;
use super::*;

pub(super) struct PreparedTurnRequest {
    pub(super) active_model: String,
    pub(super) request: LlmRequest,
    pub(super) request_body_bytes: usize,
    pub(super) large_retry_body_bytes: usize,
    pub(super) trigger_tokens: u64,
    pub(super) context_window: u64,
}

pub(super) struct StreamRound {
    pub(super) text_content: String,
    thinking_content: String,
    thinking_signature: String,
    pub(super) pending_tools: Vec<PendingToolCall>,
    pub(super) usage_acc: UsageAccumulator,
}

pub(super) struct TurnUsage {
    pub(super) turn_input_tokens: u64,
    pub(super) turn_output_tokens: u64,
    pub(super) turn_cache_creation: u64,
    pub(super) turn_cache_read: u64,
}

pub(super) enum StreamOpenOutcome {
    Stream(Receiver<StreamEvent>),
}

pub(super) enum StreamRoundOutcome {
    Completed(StreamRound),
    RetryAgentLoop,
}

pub(super) enum ToolLoopAction {
    Continue,
    Break,
}

impl Agent {
    pub(super) async fn begin_process_turn(&mut self, user_input: &str) {
        self.turn_number += 1;
        self.fire_before_agent_run_hook(user_input).await;
        self.emit_activity(
            AgentActivityKind::ParentTurnStarted,
            AgentActivityStatus::Running,
            format!("turn {} started", self.turn_number),
        );
        self.state.add_user_message(user_input);
        self.classify_cognitive_situation(user_input);
        self.spawn_auto_extraction();
    }

    pub(super) async fn prepare_turn_request(
        &mut self,
        user_input: &str,
        agentic_iterations: u32,
    ) -> Result<PreparedTurnRequest, AgentLoopError> {
        self.fire_before_prompt_build_hook(agentic_iterations).await;
        let mut system = self.inject_memories();
        self.inject_inner_voice(&mut system).await;
        self.inject_critical_reminder(&mut system);

        let active_model = self.active_model().await;
        let effort = self.turn_effort(user_input).await;
        let (max_tokens, thinking, speed) = self.config.build_base_request_fields(&active_model);
        self.maybe_auto_compact(&active_model).await?;

        let request = LlmRequest {
            model: active_model.clone(),
            max_tokens,
            system,
            messages: self.state.messages.clone(),
            tools: self.config.tools.clone(),
            thinking,
            speed,
            effort,
            extra: self.config.runtime_context_extra(),
            request_origin: Some("main_session".into()),
            reasoning_encrypted: None,
        };
        self.fire_after_prompt_build_hook(&request, agentic_iterations)
            .await;

        let request_body_bytes = autocompact::request_body_bytes(&request);
        let large_retry_body_bytes =
            autocompact::large_request_retry_body_bytes(&self.config.context);
        let trigger_tokens = if self.state.last_known_context_tokens > 0 {
            self.state.last_known_context_tokens
        } else {
            autocompact::trigger_tokens(&self.state.messages)
        };
        let context_window = self.context_window_for(&active_model);

        Ok(PreparedTurnRequest {
            active_model,
            request,
            request_body_bytes,
            large_retry_body_bytes,
            trigger_tokens,
            context_window,
        })
    }

    pub(super) async fn emit_turn_request_started(&mut self, prepared: &PreparedTurnRequest) {
        let telemetry = self.compaction_telemetry_for(&prepared.active_model);
        self.send_event(AgentEvent::ContextPressureUpdated {
            tokens_used: autocompact::approx_tokens_from_bytes(prepared.request_body_bytes),
            context_window: prepared.context_window,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            context_name: Some("main".to_string()),
            resolution_source: Some(telemetry.context_source.to_string()),
        })
        .await;
        self.send_event(AgentEvent::ApiCallStarted {
            model: prepared.active_model.clone(),
        })
        .await;
    }

    pub(super) async fn open_turn_stream(
        &mut self,
        prepared: &PreparedTurnRequest,
        reactive_overflow_retried: &mut bool,
        reactive_rate_limit_retried: &mut bool,
    ) -> Result<StreamOpenOutcome, AgentLoopError> {
        match self.client.stream(prepared.request.clone()).await {
            Ok(rx) => Ok(StreamOpenOutcome::Stream(rx)),
            Err(e) if e.is_context_window_exceeded() => {
                *reactive_overflow_retried = true;
                self.force_reactive_compact().await?;
                self.retry_stream(prepared, "reactive compaction retry failed")
                    .await
                    .map(StreamOpenOutcome::Stream)
            }
            Err(e)
                if autocompact::is_rate_limited_error(&e)
                    && !*reactive_rate_limit_retried
                    && prepared.request_body_bytes >= prepared.large_retry_body_bytes =>
            {
                *reactive_rate_limit_retried = true;
                self.warn_large_rate_limit(prepared, "rate_limit_large_request");
                self.force_reactive_compact().await?;
                self.retry_stream(prepared, "rate-limit compaction retry failed")
                    .await
                    .map(StreamOpenOutcome::Stream)
            }
            Err(e) => {
                self.fail_parent_turn(format!("{e}")).await;
                Err(AgentLoopError::ApiError(format!("{e}")))
            }
        }
    }

    pub(super) async fn collect_stream_round(
        &mut self,
        mut rx: Receiver<StreamEvent>,
        prepared: &PreparedTurnRequest,
        reactive_overflow_retried: &mut bool,
        reactive_rate_limit_retried: &mut bool,
    ) -> Result<StreamRoundOutcome, AgentLoopError> {
        let mut round = StreamRound::default();
        let mut pending_tool_indices: Vec<u32> = Vec::new();
        while let Some(event) = rx.recv().await {
            round.usage_acc.record_event(&event);
            match event {
                StreamEvent::MessageStart { .. }
                | StreamEvent::ContentBlockStop { .. }
                | StreamEvent::MessageStop
                | StreamEvent::Ping
                | StreamEvent::ReasoningEncrypted { .. } => {}
                StreamEvent::ContentBlockStart {
                    index,
                    block_type,
                    tool_use_id,
                    tool_name,
                } => {
                    if block_type == archon_llm::types::ContentBlockType::ToolUse {
                        self.record_pending_tool_start(
                            &mut round.pending_tools,
                            &mut pending_tool_indices,
                            index,
                            tool_use_id,
                            tool_name,
                        )
                        .await;
                    }
                }
                StreamEvent::TextDelta { text, .. } => {
                    round.text_content.push_str(&text);
                    self.send_event(AgentEvent::TextDelta(text)).await;
                }
                StreamEvent::ThinkingDelta { thinking, .. } => {
                    round.thinking_content.push_str(&thinking);
                    self.send_event(AgentEvent::ThinkingDelta(thinking)).await;
                }
                StreamEvent::InputJsonDelta {
                    index,
                    partial_json,
                } => append_tool_input_delta(
                    &mut round.pending_tools,
                    &pending_tool_indices,
                    index,
                    &partial_json,
                ),
                StreamEvent::MessageDelta { .. } => {}
                StreamEvent::SignatureDelta { signature, .. } => {
                    round.thinking_signature.push_str(&signature);
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    return self
                        .handle_stream_error(
                            error_type,
                            message,
                            prepared,
                            reactive_overflow_retried,
                            reactive_rate_limit_retried,
                        )
                        .await;
                }
            }
        }
        Ok(StreamRoundOutcome::Completed(round))
    }

    pub(super) fn record_stream_usage(&mut self, usage: &UsageAccumulator) -> TurnUsage {
        self.state.total_input_tokens += usage.context_input_tokens;
        self.state.last_known_context_tokens = usage.context_input_tokens;
        self.state.total_output_tokens += usage.output_tokens;
        TurnUsage {
            turn_input_tokens: usage.billable_input_tokens,
            turn_output_tokens: usage.output_tokens,
            turn_cache_creation: usage.cache_creation_input_tokens,
            turn_cache_read: usage.cache_read_input_tokens,
        }
    }

    pub(super) fn add_assistant_stream_round(&mut self, round: &StreamRound) {
        let mut assistant_content = Vec::new();
        if !round.thinking_content.is_empty() {
            assistant_content.push(serde_json::json!({
                "type": "thinking",
                "thinking": round.thinking_content,
                "signature": round.thinking_signature,
            }));
        }
        if !round.text_content.is_empty() {
            assistant_content.push(serde_json::json!({
                "type": "text",
                "text": round.text_content,
            }));
        }
        for tool in &round.pending_tools {
            assistant_content.push(self.assistant_tool_use_block(tool));
        }
        self.state.add_assistant_message(assistant_content);
    }
}

impl Default for StreamRound {
    fn default() -> Self {
        Self {
            text_content: String::new(),
            thinking_content: String::new(),
            thinking_signature: String::new(),
            pending_tools: Vec::new(),
            usage_acc: UsageAccumulator::default(),
        }
    }
}
