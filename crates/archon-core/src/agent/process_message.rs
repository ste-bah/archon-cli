use super::process_message_steps::{StreamOpenOutcome, StreamRoundOutcome, ToolLoopAction};
use super::*;

impl Agent {
    /// Process a single user message through the full agent loop.
    /// Returns when the LLM produces a final text response (no more tool calls).
    pub async fn process_message(&mut self, user_input: &str) -> Result<(), AgentLoopError> {
        self.begin_process_turn(user_input).await;
        if self.try_complete_trivial_cognitive_turn(user_input).await {
            self.emit_activity(
                AgentActivityKind::ParentTurnCompleted,
                AgentActivityStatus::Completed,
                format!("turn {} completed", self.turn_number),
            );
            self.fire_after_agent_run_hook("completed", None).await;
            return Ok(());
        }

        let mut agentic_iterations: u32 = 0;
        let mut reactive_overflow_retried = false;
        let mut reactive_rate_limit_retried = false;
        let mut proactive_pressure_attempted = false;
        'agent_loop: loop {
            let prepared = self
                .prepare_turn_request(user_input, agentic_iterations)
                .await?;
            if !proactive_pressure_attempted
                && self
                    .maybe_request_pressure_compact(
                        &prepared.active_model,
                        prepared.trigger_tokens,
                        prepared.request_body_bytes,
                        prepared.context_window,
                    )
                    .await?
            {
                proactive_pressure_attempted = true;
                continue 'agent_loop;
            }

            self.emit_turn_request_started(&prepared).await;
            let rx = match self
                .open_turn_stream(
                    &prepared,
                    &mut reactive_overflow_retried,
                    &mut reactive_rate_limit_retried,
                )
                .await?
            {
                StreamOpenOutcome::Stream(rx) => rx,
            };

            let round = match self
                .collect_stream_round(
                    rx,
                    &prepared,
                    &mut reactive_overflow_retried,
                    &mut reactive_rate_limit_retried,
                )
                .await?
            {
                StreamRoundOutcome::Completed(round) => round,
                StreamRoundOutcome::RetryAgentLoop => continue 'agent_loop,
            };
            reactive_overflow_retried = false;
            reactive_rate_limit_retried = false;

            let usage = self.record_stream_usage(&round.usage_acc);
            self.add_assistant_stream_round(&round);
            self.emit_reasoning_turn(&round.text_content);

            if !round.pending_tools.is_empty() {
                match self
                    .handle_pending_tool_round(
                        &round.pending_tools,
                        &prepared.active_model,
                        &mut agentic_iterations,
                    )
                    .await
                {
                    ToolLoopAction::Continue => continue 'agent_loop,
                    ToolLoopAction::Break => break,
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
