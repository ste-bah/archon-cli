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
    pub(super) context_window: u64,
}

#[derive(Default)]
pub(super) struct StreamRound {
    pub(super) text_content: String,
    pub(super) thinking_content: String,
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
    RetryStream(Receiver<StreamEvent>),
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
        self.run_cognitive_executive_advisory().await;
        // Issue #76: the full executive loop runs here as a shadow observer,
        // before the LLM request, so its plan is on record ahead of the live
        // turn instead of being reconstructed afterwards. It executes nothing
        // and is bounded by the cognitive pipeline budget.
        self.run_cognitive_shadow_observation(user_input).await;
        // Issues #80/#81: record the self-model's pre-action prediction for
        // this turn and select the bounded set of unresolved lessons the prompt
        // will carry. Both happen before the request is built, so the
        // prediction is on record ahead of the action it predicts.
        self.prepare_cognitive_turn_learning().await;
        self.spawn_auto_extraction();
    }

    pub(super) async fn prepare_turn_request(
        &mut self,
        user_input: &str,
        agentic_iterations: u32,
    ) -> Result<PreparedTurnRequest, AgentLoopError> {
        self.fire_before_prompt_build_hook(agentic_iterations).await;
        // Everything `inject_memories` returns beyond the configured blocks, and
        // everything appended after it, is rebuilt per turn. Recording the
        // boundary here is what lets the stable head be cached separately from
        // the churn — see `apply_stable_system_cache`.
        let mut stable_system_blocks = self.config.system_prompt.len();
        let mut system = self.inject_memories().await;
        self.inject_inner_voice(&mut system).await;
        self.inject_critical_reminder(&mut system);
        self.inject_turn_requirements(&mut system);

        let active_model = self.active_model().await;
        let effort = self.turn_effort(user_input).await;
        // #123: `ultrathink` escalates BOTH knobs for this turn — effort to
        // `Max` (above, inside `turn_effort`) and the thinking budget here.
        // The two are resolved from the same `user_input` so they cannot
        // disagree about whether the turn was escalated.
        let ultrathink = archon_llm::thinking::ultrathink_requested(user_input);
        let (max_tokens, thinking, speed) = self
            .config
            .build_base_request_fields_with(&active_model, ultrathink);
        let mut messages = self.messages_for_turn_request(&active_model)?;

        // #178: the blocks appended above change almost every turn and sit in
        // front of the tools and the entire history, so they invalidate the
        // prefix for every provider. Moving them onto the last user turn — where
        // archon's own reminders already go — leaves one uninterrupted prefix.
        // On the implicitly-caching providers this is the only lever there is;
        // there is no breakpoint to place.
        if self.config.context.prompt_cache_reorder {
            let moved = prompt_ordering::move_volatile_system_to_last_turn(
                &mut system,
                &mut messages,
                stable_system_blocks,
            );
            if moved > 0 {
                // Everything left is stable, so there is no boundary inside the
                // system prompt any more and a second breakpoint there would be
                // spent for nothing.
                stable_system_blocks = system.len();
            }
        }

        let mut request = LlmRequest {
            model: active_model.clone(),
            max_tokens,
            system,
            messages,
            // The main-agent loop keeps its owned tool list (#171 non-goal:
            // no main-agent changes); it is wrapped here so the request type
            // stays shared-by-construction for the subagent path.
            tools: archon_llm::provider::shared_tools(self.config.tools.clone()),
            thinking,
            speed,
            effort,
            extra: self.config.request_runtime_extra(
                self.turn_number,
                agentic_iterations,
                self.context_window_for(&active_model),
            ),
            request_origin: Some("main_session".into()),
            reasoning_encrypted: None,
        };
        request_cache::apply_stable_system_cache(
            &mut request,
            self.client.as_ref(),
            stable_system_blocks,
            &self.config.context.prompt_cache_strategy,
            self.config.context.prompt_cache,
            &self.config.context.prompt_cache_mode,
            &self.config.context.prompt_cache_ttl,
            &self.config.context.prompt_cache_models,
        );
        request_cache::apply_conversation_cache(
            &mut request,
            self.client.as_ref(),
            &self.config.context.prompt_cache_strategy,
            self.config.context.prompt_cache,
            self.config.context.prompt_cache_conversation,
            &self.config.context.prompt_cache_mode,
            &self.config.context.prompt_cache_ttl,
            &self.config.context.prompt_cache_models,
        );
        self.fire_after_prompt_build_hook(&request, agentic_iterations)
            .await;

        let request_body_bytes = autocompact::request_body_bytes(&request);
        let large_retry_body_bytes =
            autocompact::large_request_retry_body_bytes(&self.config.context);
        let context_window = self.context_window_for(&active_model);

        Ok(PreparedTurnRequest {
            active_model,
            request,
            request_body_bytes,
            large_retry_body_bytes,
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
        recovery_ladder: &mut autocompact::RecoveryLadder,
        reactive_rate_limit_retried: &mut bool,
    ) -> Result<StreamOpenOutcome, AgentLoopError> {
        match self.client.stream(prepared.request.clone()).await {
            Ok(rx) => Ok(StreamOpenOutcome::Stream(rx)),
            Err(e)
                if autocompact::request_pressure_kind_for_request(&e, &prepared.request)
                    .is_some() =>
            {
                let classification =
                    autocompact::request_pressure_kind_for_request(&e, &prepared.request)
                        .expect("guarded pressure kind");
                self.recover_request_pressure(prepared, recovery_ladder, classification)
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
                let messages = self.force_reactive_compact().await?;
                self.retry_stream_with_messages(
                    prepared,
                    messages,
                    "rate-limit compaction retry failed",
                )
                .await
                .map(StreamOpenOutcome::Stream)
            }
            Err(e) => {
                self.discard_thinking_preview().await;
                self.fail_parent_turn(format!("{e}")).await;
                Err(AgentLoopError::ApiError(format!("{e}")))
            }
        }
    }

    pub(super) async fn collect_stream_round(
        &mut self,
        mut rx: Receiver<StreamEvent>,
        prepared: &PreparedTurnRequest,
        recovery_ladder: &mut autocompact::RecoveryLadder,
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
                    if !self.buffers_finalization_text() {
                        self.send_event(AgentEvent::TextDelta(text)).await;
                    }
                }
                StreamEvent::ThinkingDelta { thinking, .. } => {
                    round.thinking_content.push_str(&thinking);
                    let event = if self.buffers_finalization_text() {
                        AgentEvent::TransientThinkingDelta(thinking)
                    } else {
                        AgentEvent::ThinkingDelta(thinking)
                    };
                    self.send_event(event).await;
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
                            recovery_ladder,
                            reactive_rate_limit_retried,
                        )
                        .await;
                }
            }
        }
        Ok(StreamRoundOutcome::Completed(round))
    }

    pub(super) fn record_stream_usage(&mut self, usage: &UsageAccumulator) -> TurnUsage {
        // The single point where a stream's accumulated usage becomes the
        // numbers the status bar and the cost estimate are built from. Logged
        // because a zero here and a zero at the provider look identical from
        // the outside, and telling them apart is the whole diagnosis when a
        // cache appears not to be working.
        tracing::debug!(
            billable_input = usage.billable_input_tokens,
            context_input = usage.context_input_tokens,
            output = usage.output_tokens,
            cache_creation = usage.cache_creation_input_tokens,
            cache_read = usage.cache_read_input_tokens,
            "turn usage accumulated"
        );
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
        self.insert_assistant_stream_round(self.state.messages.len(), round, true);
    }

    pub(super) fn insert_assistant_stream_round(
        &mut self,
        index: usize,
        round: &StreamRound,
        include_drafts: bool,
    ) {
        let mut assistant_content = self.stream_round_drafts(round, include_drafts);
        for tool in &round.pending_tools {
            assistant_content.push(self.assistant_tool_use_block(tool));
        }
        self.state.messages.insert(
            index,
            serde_json::json!({
                "role": "assistant",
                "content": assistant_content,
            }),
        );
    }

    pub(super) fn prepend_stream_round_drafts(&mut self, index: usize, round: &StreamRound) {
        let drafts = self.stream_round_drafts(round, true);
        let Some(content) = self.state.messages[index]["content"].as_array_mut() else {
            return;
        };
        content.splice(0..0, drafts);
    }

    pub(super) fn persist_guarded_plan_after_draft_admission(&self, round: &StreamRound) {
        let exited_plan = round
            .pending_tools
            .iter()
            .any(|tool| tool.name == "ExitPlanMode" && self.tool_result_passed(&tool.id));
        if exited_plan {
            self.persist_latest_plan_from_assistant();
        }
    }

    fn tool_result_passed(&self, tool_id: &str) -> bool {
        self.state.messages.iter().rev().any(|message| {
            message["content"].as_array().is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    block["type"] == "tool_result"
                        && block["tool_use_id"] == tool_id
                        && block["is_error"] == false
                })
            })
        })
    }

    fn stream_round_drafts(
        &self,
        round: &StreamRound,
        include_drafts: bool,
    ) -> Vec<serde_json::Value> {
        if !include_drafts {
            return Vec::new();
        }
        let mut drafts = Vec::new();
        if !round.thinking_content.is_empty() {
            drafts.push(serde_json::json!({
                "type": "thinking",
                "thinking": round.thinking_content,
                "signature": round.thinking_signature,
            }));
        }
        if !round.text_content.is_empty() {
            drafts.push(serde_json::json!({
                "type": "text",
                "text": round.text_content,
            }));
        }
        drafts
    }
}
