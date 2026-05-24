use super::*;

mod stream_round;
mod tool_round;

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

            let first_user_message = messages
                .first()
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("");

            let mut system: Vec<serde_json::Value> = Vec::new();

            if let Some(billing) = self.identity.billing_header(first_user_message) {
                system.push(serde_json::json!({
                    "type": "text",
                    "text": billing,
                    "cache_control": { "type": "ephemeral" }
                }));
            }

            system.push(serde_json::json!({
                "type": "text",
                "text": &self.system_prompt,
            }));

            // Inject critical system reminder every turn (AGT-022)
            if let Some(ref reminder) = self.critical_system_reminder {
                system.push(serde_json::json!({
                    "type": "text",
                    "text": format!("<system-reminder>{reminder}</system-reminder>"),
                }));
            }

            // Effort layering: per-agent-definition override wins if Some;
            // otherwise read the parent's live /effort setting (v0.1.18).
            let effort = if self.effort.is_some() {
                self.effort.clone()
            } else {
                let level = self.agent_config.effort_level.lock().await;
                match *level {
                    archon_llm::effort::EffortLevel::High => None,
                    other => Some(other.to_string()),
                }
            };

            let (max_tokens, thinking, speed) =
                self.agent_config.build_base_request_fields(&self.model);

            let resolved_window = archon_llm::context_window::resolve_context_window_for_work_dir(
                &self.model,
                self.agent_config
                    .context
                    .context_window_override
                    .or_else(|| self.agent_config.context.max_tokens.map(u64::from)),
                Some(self.provider.as_ref()),
                Some(&self.agent_config.working_dir),
            );
            let window = resolved_window
                .runtime_context_budget
                .unwrap_or(resolved_window.context_window);
            let telemetry = crate::agent::autocompact::CompactionTelemetry {
                provider_family: self.provider.compaction_policy().provider_family.label(),
                wire_shape: self.provider.compaction_policy().wire_shape.label(),
                native_context_window: resolved_window.context_window,
                runtime_context_budget: window,
                context_source: resolved_window.source.label(),
                compaction_backend: self.provider.compaction_policy().backend.label(),
            };
            let effective_window =
                window.saturating_sub(self.agent_config.context.output_reserve_tokens);
            let threshold = (self.agent_config.context.compact_threshold
                - self.agent_config.context.preflight_safety_margin)
                .max(0.0);
            if let Some(action) = crate::agent::evaluate_compaction(
                if last_known_context_tokens > 0 {
                    last_known_context_tokens
                } else {
                    crate::agent::autocompact::trigger_tokens(&messages)
                },
                effective_window,
                &auto_compact,
                threshold,
            ) {
                tracing::info!(
                    compaction.reason = "context_window_threshold",
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    scope = "subagent",
                    force = false,
                    consecutive_failures = auto_compact.consecutive_failures,
                    "subagent auto-compaction attempt started"
                );
                auto_compact.compact_in_flight = true;
                match crate::agent::autocompact::compact_json_messages_with_provider(
                    self.provider.as_ref(),
                    &self.model,
                    &messages,
                    action,
                    false,
                )
                .await
                {
                    Ok((
                        crate::agent::autocompact::CompactionOutcome::Compacted {
                            after_estimated_tokens,
                            ..
                        },
                        compacted,
                    )) => {
                        messages = compacted;
                        last_known_context_tokens = 0;
                        auto_compact.on_success(after_estimated_tokens);
                    }
                    Ok((crate::agent::autocompact::CompactionOutcome::Skipped { .. }, _)) => {
                        auto_compact.on_cancel();
                    }
                    Err(crate::agent::autocompact::CompactionError::Cancelled) => {
                        auto_compact.on_cancel();
                        tracing::debug!(
                            compaction.outcome = "cancelled",
                            provider_family = telemetry.provider_family,
                            wire_shape = telemetry.wire_shape,
                            native_context_window = telemetry.native_context_window,
                            runtime_context_budget = telemetry.runtime_context_budget,
                            context_source = telemetry.context_source,
                            compaction_backend = telemetry.compaction_backend,
                            actor = %self.activity_actor_id
                                .as_deref().unwrap_or("subagent"), "proactive subagent compaction cancelled"
                        );
                    }
                    Err(e) => {
                        auto_compact.on_real_failure();
                        tracing::warn!(
                            compaction.outcome = "auto_failed",
                            provider_family = telemetry.provider_family,
                            wire_shape = telemetry.wire_shape,
                            native_context_window = telemetry.native_context_window,
                            runtime_context_budget = telemetry.runtime_context_budget,
                            context_source = telemetry.context_source,
                            compaction_backend = telemetry.compaction_backend,
                            actor = %self.activity_actor_id
                                .as_deref().unwrap_or("subagent"), consecutive_failures =
                                auto_compact.consecutive_failures, breaker_tripped =
                                auto_compact.disabled, error = %e,
                            "proactive subagent compaction failed; continuing turn",
                        );
                    }
                }
            }

            let mut request = LlmRequest {
                model: self.model.clone(),
                max_tokens,
                system,
                messages: messages.clone(),
                tools: self.tool_definitions.clone(),
                thinking,
                speed,
                effort,
                extra: serde_json::Value::Null,
                request_origin: Some("subagent".into()),
                reasoning_encrypted: reasoning_encrypted.clone(),
            };
            let mut request_body_bytes = crate::agent::autocompact::request_body_bytes(&request);
            let large_retry_body_bytes = crate::agent::autocompact::large_request_retry_body_bytes(
                &self.agent_config.context,
            );
            let trigger_tokens = if last_known_context_tokens > 0 {
                last_known_context_tokens
            } else {
                crate::agent::autocompact::trigger_tokens(&messages)
            };
            let token_pressure = self
                .agent_config
                .context
                .rate_limit_pressure_tokens
                .is_some_and(|threshold| trigger_tokens >= threshold);
            let body_pressure = self
                .agent_config
                .context
                .rate_limit_pressure_body_bytes
                .is_some_and(|threshold| request_body_bytes as u64 >= threshold);
            if !proactive_pressure_attempted
                && (token_pressure || body_pressure)
                && auto_compact.should_attempt()
            {
                proactive_pressure_attempted = true;
                let reason = match (token_pressure, body_pressure) {
                    (true, true) => "request_pressure_tokens_and_bytes",
                    (true, false) => "request_pressure_tokens",
                    (false, true) => "request_pressure_bytes",
                    (false, false) => unreachable!(),
                };
                tracing::info!(
                    compaction.reason = reason,
                    trigger_tokens,
                    trigger_body_bytes = request_body_bytes,
                    context_window = window,
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    scope = "subagent",
                    force = false,
                    consecutive_failures = auto_compact.consecutive_failures,
                    "subagent request pressure threshold reached; attempting proactive compaction"
                );
                auto_compact.compact_in_flight = true;
                match crate::agent::autocompact::compact_json_messages_with_provider(
                    self.provider.as_ref(),
                    &self.model,
                    &messages,
                    crate::agent::CompactAction::Full,
                    false,
                )
                .await
                {
                    Ok((
                        crate::agent::autocompact::CompactionOutcome::Compacted {
                            after_estimated_tokens,
                            ..
                        },
                        compacted,
                    )) => {
                        messages = compacted;
                        request.messages = messages.clone();
                        request_body_bytes =
                            crate::agent::autocompact::request_body_bytes(&request);
                        last_known_context_tokens = 0;
                        auto_compact.on_success(after_estimated_tokens);
                    }
                    Ok((crate::agent::autocompact::CompactionOutcome::Skipped { .. }, _)) => {
                        auto_compact.on_cancel();
                    }
                    Err(crate::agent::autocompact::CompactionError::Cancelled) => {
                        auto_compact.on_cancel();
                    }
                    Err(e) => {
                        auto_compact.on_real_failure();
                        tracing::warn!(
                            compaction.reason = reason,
                            trigger_tokens,
                            trigger_body_bytes = request_body_bytes,
                            context_window = window,
                            provider_family = telemetry.provider_family,
                            wire_shape = telemetry.wire_shape,
                            native_context_window = telemetry.native_context_window,
                            runtime_context_budget = telemetry.runtime_context_budget,
                            context_source = telemetry.context_source,
                            compaction_backend = telemetry.compaction_backend,
                            scope = "subagent",
                            force = false,
                            consecutive_failures = auto_compact.consecutive_failures,
                            breaker_tripped = auto_compact.disabled,
                            error = %e,
                            "subagent request-pressure compaction failed; continuing turn",
                        );
                    }
                }
            }

            let stream = collect_stream_round(
                self,
                &mut messages,
                &mut auto_compact,
                &mut reactive_overflow_retried,
                &mut reactive_rate_limit_retried,
                &mut last_known_context_tokens,
                request,
                request_body_bytes,
                large_retry_body_bytes,
                &telemetry,
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
