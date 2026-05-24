use super::*;

mod request_round;
mod stream_round;
mod tool_round;

use request_round::prepare_request_round;
use stream_round::collect_stream_round;
use tool_round::replay_tool_round;

impl SubagentRunner {
    /// Run the subagent loop with the given initial prompt.
    /// Returns the accumulated text output from the final turn.
    pub async fn run(&self, initial_prompt: &str) -> anyhow::Result<String> {
        // AGT-024: Use initial_messages for resume, or start fresh
        let mut messages: Vec<serde_json::Value> = if let Some(ref initial) = self.initial_messages
        {
            let mut msgs = initial.clone();
            let user_msg = serde_json::json!({
                "role": "user",
                "content": initial_prompt,
            });
            self.record_transcript(&user_msg);
            msgs.push(user_msg);
            msgs
        } else {
            let user_msg = serde_json::json!({
                "role": "user",
                "content": initial_prompt,
            });
            self.record_transcript(&user_msg);
            vec![user_msg]
        };

        let started = Instant::now();
        let deadline = started + Duration::from_secs(self.timeout_secs);
        let mut auto_compact = crate::agent::AutoCompactState::default();
        let mut cumulative_billable_tokens = 0_u64;
        let mut last_known_context_tokens = 0_u64;
        let mut reasoning_encrypted: Option<String> = None;
        let mut reactive_overflow_retried = false;
        let mut reactive_rate_limit_retried = false;
        let mut proactive_pressure_attempted = false;

        for turn in 0..self.max_turns {
            // Check timeout. The error message reports BOTH wall-clock
            // elapsed and turn counter so an LLM (or human) reading
            // the failure can tell which cap actually fired — the
            // pre-v0.1.42 message ("Subagent timed out after N turns")
            // misled both LLMs and reviewers into thinking N was a
            // turn cap when it was always a wall-clock cap. Default
            // wall-clock is now 24h (DEFAULT_TIMEOUT_SECS = 86400).
            if Instant::now() >= deadline {
                let elapsed = started.elapsed().as_secs();
                anyhow::bail!(
                    "Subagent wall-clock timeout: {elapsed}s elapsed (cap: {}s) at turn {}/{} — \
                     override per-spawn with timeout_secs:<seconds>, or per-agent in frontmatter",
                    self.timeout_secs,
                    turn,
                    self.max_turns,
                );
            }

            // Check for graceful shutdown request
            if self
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Ok("[Agent shutdown requested]".to_string());
            }

            let prepared_request = prepare_request_round(
                self,
                &mut messages,
                &mut auto_compact,
                &mut last_known_context_tokens,
                &mut proactive_pressure_attempted,
                reasoning_encrypted.clone(),
            )
            .await;
            let stream = collect_stream_round(
                self,
                &mut messages,
                &mut auto_compact,
                &mut reactive_overflow_retried,
                &mut reactive_rate_limit_retried,
                &mut last_known_context_tokens,
                prepared_request.request,
                prepared_request.request_body_bytes,
                prepared_request.large_retry_body_bytes,
                &prepared_request.telemetry,
            )
            .await?;
            if stream.retry_after_compact {
                continue;
            }
            reasoning_encrypted = stream.reasoning_encrypted;
            reactive_overflow_retried = false;
            reactive_rate_limit_retried = false;
            cumulative_billable_tokens += stream.context_input_tokens;
            last_known_context_tokens = stream.context_input_tokens;
            tracing::trace!(cumulative_billable_tokens, "subagent billable input tokens");

            // If no tool calls, subagent is done — return accumulated text
            if stream.pending_tools.is_empty() {
                // Record final assistant text to transcript (AGT-024)
                if !stream.text_content.is_empty() {
                    self.record_transcript(&serde_json::json!({
                        "role": "assistant",
                        "content": stream.text_content,
                    }));
                }
                self.emit_activity_stream("final", "subagent turn complete", None, false);
                return Ok(stream.text_content);
            }

            replay_tool_round(
                self,
                &mut messages,
                stream.text_content,
                stream.thinking_blocks,
                stream.pending_tools,
            )
            .await;
        }

        self.emit_activity_stream(
            "error",
            format!("Subagent reached max turns ({})", self.max_turns),
            None,
            true,
        );
        anyhow::bail!("Subagent reached max turns ({})", self.max_turns)
    }
}

fn summarize_tool_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.chars().count() <= 500 {
        return trimmed.to_string();
    }
    let mut summary: String = trimmed.chars().take(500).collect();
    summary.push_str("...");
    summary
}
