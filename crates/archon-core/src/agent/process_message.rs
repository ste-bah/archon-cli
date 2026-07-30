use super::process_message_steps::{
    StreamOpenOutcome, StreamRound, StreamRoundOutcome, ToolLoopAction,
};
use super::*;

impl Agent {
    pub(super) fn buffers_finalization_text(&self) -> bool {
        self.turn_finalization_callback.is_some() && self.guardrail_action_id.is_some()
    }

    pub(super) async fn commit_thinking_preview(&self) {
        if self.buffers_finalization_text() {
            self.send_event(AgentEvent::CommitThinkingPreview).await;
        }
    }

    pub(super) async fn discard_thinking_preview(&self) {
        if self.buffers_finalization_text() {
            self.send_event(AgentEvent::DiscardThinkingPreview).await;
        }
    }

    async fn emit_buffered_round_output(&self, text: &str) {
        if !self.buffers_finalization_text() {
            return;
        }
        if !text.is_empty() {
            self.send_event(AgentEvent::TextDelta(text.to_owned()))
                .await;
        }
    }

    async fn admit_tool_round_drafts(&mut self, message_index: usize, round: &StreamRound) {
        self.prepend_stream_round_drafts(message_index, round);
        self.persist_guarded_plan_after_draft_admission(round);
        self.emit_reasoning_turn(&round.text_content);
        self.commit_thinking_preview().await;
        self.emit_buffered_round_output(&round.text_content).await;
    }

    fn finalization_verdict(&self, output: &str) -> TurnFinalizationVerdict {
        let Some(callback) = &self.turn_finalization_callback else {
            return TurnFinalizationVerdict::Allowed;
        };
        callback(
            self.guardrail_action_id.as_deref().unwrap_or_default(),
            output,
        )
    }

    pub(super) async fn finalize_tool_loop_break(
        &mut self,
        output: &str,
    ) -> Result<(), AgentLoopError> {
        if !self.buffers_finalization_text() {
            return Ok(());
        }
        match self.finalization_verdict(output) {
            TurnFinalizationVerdict::Allowed => Ok(()),
            TurnFinalizationVerdict::Blocked { repair_prompt } => {
                self.discard_thinking_preview().await;
                self.fail_parent_turn(repair_prompt.clone()).await;
                Err(AgentLoopError::FinalizationBlocked(repair_prompt))
            }
        }
    }

    /// Process a single user message through the full agent loop.
    /// Returns when the LLM produces a final text response (no more tool calls).
    pub async fn process_message(&mut self, user_input: &str) -> Result<(), AgentLoopError> {
        self.begin_process_turn(user_input).await;
        let trivial_response = self.try_complete_trivial_cognitive_turn().await;
        let mut agentic_iterations: u32 = 0;
        let mut recovery_ladder = autocompact::RecoveryLadder::default();
        let mut reactive_rate_limit_retried = false;
        let mut finalization_repair_attempted = false;
        if let Some(response) = trivial_response {
            match self.finalization_verdict(&response) {
                TurnFinalizationVerdict::Allowed => {
                    self.state.add_assistant_message(vec![serde_json::json!({
                        "type": "text",
                        "text": response,
                    })]);
                    self.emit_buffered_round_output(&response).await;
                    let active_model = self.active_model().await;
                    self.complete_turn_without_tools(user_input, 0, 0, 0, 0, &active_model)
                        .await;
                    self.emit_activity(
                        AgentActivityKind::ParentTurnCompleted,
                        AgentActivityStatus::Completed,
                        format!("turn {} completed", self.turn_number),
                    );
                    self.fire_after_agent_run_hook("completed", None).await;
                    return Ok(());
                }
                TurnFinalizationVerdict::Blocked { repair_prompt } => {
                    finalization_repair_attempted = true;
                    self.state.add_user_message(&repair_prompt);
                }
            }
        }

        'agent_loop: loop {
            let prepared = self
                .prepare_turn_request(user_input, agentic_iterations)
                .await?;
            self.emit_turn_request_started(&prepared).await;
            let StreamOpenOutcome::Stream(rx) = self
                .open_turn_stream(
                    &prepared,
                    &mut recovery_ladder,
                    &mut reactive_rate_limit_retried,
                )
                .await?;

            let mut round_rx = rx;
            let round = loop {
                match self
                    .collect_stream_round(
                        round_rx,
                        &prepared,
                        &mut recovery_ladder,
                        &mut reactive_rate_limit_retried,
                    )
                    .await?
                {
                    StreamRoundOutcome::Completed(round) => break round,
                    StreamRoundOutcome::RetryStream(retry_rx) => round_rx = retry_rx,
                }
            };
            recovery_ladder = autocompact::RecoveryLadder::default();
            reactive_rate_limit_retried = false;

            let usage = self.record_stream_usage(&round.usage_acc);
            self.state.auto_compact.on_ordinary_success();

            if !round.pending_tools.is_empty() {
                let assistant_message_index = self.state.messages.len();
                self.insert_assistant_stream_round(assistant_message_index, &round, false);
                match self
                    .handle_pending_tool_round(
                        &round.pending_tools,
                        &prepared.active_model,
                        &mut agentic_iterations,
                    )
                    .await
                {
                    ToolLoopAction::Continue => {
                        self.admit_tool_round_drafts(assistant_message_index, &round)
                            .await;
                        continue 'agent_loop;
                    }
                    ToolLoopAction::Break => {
                        self.finalize_tool_loop_break(&round.text_content).await?;
                        self.admit_tool_round_drafts(assistant_message_index, &round)
                            .await;
                        break;
                    }
                }
            }

            match self.finalization_verdict(&round.text_content) {
                TurnFinalizationVerdict::Allowed => {
                    self.add_assistant_stream_round(&round);
                    self.emit_reasoning_turn(&round.text_content);
                    self.commit_thinking_preview().await;
                    self.emit_buffered_round_output(&round.text_content).await;
                }
                TurnFinalizationVerdict::Blocked { repair_prompt }
                    if !finalization_repair_attempted =>
                {
                    self.discard_thinking_preview().await;
                    finalization_repair_attempted = true;
                    self.state.add_user_message(&repair_prompt);
                    continue 'agent_loop;
                }
                TurnFinalizationVerdict::Blocked { repair_prompt } => {
                    self.discard_thinking_preview().await;
                    self.fail_parent_turn(repair_prompt.clone()).await;
                    return Err(AgentLoopError::FinalizationBlocked(repair_prompt));
                }
            }

            self.complete_turn_without_tools(
                user_input,
                usage.turn_input_tokens,
                usage.turn_output_tokens,
                usage.turn_cache_creation,
                usage.turn_cache_read,
                &prepared.active_model,
            )
            .await;
            break;
        }

        self.emit_activity(
            AgentActivityKind::ParentTurnCompleted,
            AgentActivityStatus::Completed,
            format!("turn {} completed", self.turn_number),
        );
        self.fire_after_agent_run_hook("completed", None).await;
        Ok(())
    }
}
