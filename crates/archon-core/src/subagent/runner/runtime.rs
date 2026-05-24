use super::*;

mod tool_round;

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

            // Stream the response
            let mut rx = match self.provider.stream(request.clone()).await {
                Ok(rx) => rx,
                Err(e) if e.is_context_window_exceeded() && !reactive_overflow_retried => {
                    reactive_overflow_retried = true;
                    let (outcome, compacted) =
                        crate::agent::autocompact::compact_json_messages_with_provider(
                            self.provider.as_ref(),
                            &self.model,
                            &messages,
                            crate::agent::CompactAction::Full,
                            true,
                        )
                        .await
                        .map_err(|err| {
                            anyhow::anyhow!("reactive subagent compaction failed: {err}")
                        })?;
                    messages = compacted;
                    let after_current_tokens = match outcome {
                        crate::agent::autocompact::CompactionOutcome::Compacted {
                            after_estimated_tokens,
                            ..
                        } => after_estimated_tokens,
                        crate::agent::autocompact::CompactionOutcome::Skipped { .. } => {
                            crate::agent::autocompact::estimate_messages_tokens(&messages)
                        }
                    };
                    last_known_context_tokens = 0;
                    auto_compact.on_success(after_current_tokens);
                    self.provider
                        .stream(LlmRequest {
                            messages: messages.clone(),
                            ..request
                        })
                        .await
                        .map_err(anyhow::Error::new)?
                }
                Err(e)
                    if crate::agent::autocompact::is_rate_limited_error(&e)
                        && !reactive_rate_limit_retried
                        && request_body_bytes >= large_retry_body_bytes =>
                {
                    reactive_rate_limit_retried = true;
                    tracing::warn!(
                        compaction.reason = "rate_limit_large_request",
                        trigger_body_bytes = request_body_bytes,
                        threshold_body_bytes = large_retry_body_bytes,
                        provider_family = telemetry.provider_family,
                        wire_shape = telemetry.wire_shape,
                        native_context_window = telemetry.native_context_window,
                        runtime_context_budget = telemetry.runtime_context_budget,
                        context_source = telemetry.context_source,
                        compaction_backend = telemetry.compaction_backend,
                        scope = "subagent",
                        force = true,
                        "rate-limited subagent request is large; compacting before one retry"
                    );
                    let (outcome, compacted) =
                        crate::agent::autocompact::compact_json_messages_with_provider(
                            self.provider.as_ref(),
                            &self.model,
                            &messages,
                            crate::agent::CompactAction::Full,
                            true,
                        )
                        .await
                        .map_err(|err| {
                            anyhow::anyhow!("rate-limit subagent compaction failed: {err}")
                        })?;
                    messages = compacted;
                    let after_current_tokens = match outcome {
                        crate::agent::autocompact::CompactionOutcome::Compacted {
                            after_estimated_tokens,
                            ..
                        } => after_estimated_tokens,
                        crate::agent::autocompact::CompactionOutcome::Skipped { .. } => {
                            crate::agent::autocompact::estimate_messages_tokens(&messages)
                        }
                    };
                    last_known_context_tokens = 0;
                    auto_compact.on_success(after_current_tokens);
                    self.provider
                        .stream(LlmRequest {
                            messages: messages.clone(),
                            ..request
                        })
                        .await
                        .map_err(anyhow::Error::new)?
                }
                Err(e) => return Err(anyhow::Error::new(e)),
            };

            let mut text_content = String::new();
            let mut thinking_blocks =
                std::collections::BTreeMap::<u32, PendingThinkingBlock>::new();
            let mut turn_reasoning_encrypted: Option<String> = None;
            let mut pending_tools: Vec<PendingTool> = Vec::new();
            let mut pending_tool_indices: Vec<u32> = Vec::new();
            let mut usage_acc = archon_llm::usage::UsageAccumulator::default();
            let mut retry_after_compact = false;

            while let Some(event) = rx.recv().await {
                usage_acc.record_event(&event);
                match event {
                    StreamEvent::ContentBlockStart {
                        index,
                        block_type,
                        tool_use_id,
                        tool_name,
                    } => {
                        if block_type == ContentBlockType::ToolUse {
                            let name = tool_name.clone().unwrap_or_default();
                            self.emit_activity_stream(
                                "tool_call",
                                format!("calling {name}"),
                                Some(&name),
                                false,
                            );
                            pending_tools.push(PendingTool {
                                id: tool_use_id.unwrap_or_default(),
                                name,
                                input_json: String::new(),
                            });
                            pending_tool_indices.push(index);
                        } else if block_type == ContentBlockType::Thinking {
                            thinking_blocks.entry(index).or_default();
                        }
                    }
                    StreamEvent::TextDelta { text, .. } => {
                        self.emit_activity_stream("text", text.clone(), None, false);
                        text_content.push_str(&text);
                    }
                    StreamEvent::ThinkingDelta { index, thinking } => {
                        thinking_blocks
                            .entry(index)
                            .or_default()
                            .thinking
                            .push_str(&thinking);
                        self.emit_activity_stream("thinking", thinking, None, false);
                    }
                    StreamEvent::SignatureDelta { index, signature } => {
                        thinking_blocks
                            .entry(index)
                            .or_default()
                            .signature
                            .push_str(&signature);
                    }
                    StreamEvent::ReasoningEncrypted { blob } => {
                        turn_reasoning_encrypted = Some(blob);
                    }
                    StreamEvent::InputJsonDelta {
                        index,
                        partial_json,
                    } => {
                        if !crate::agent::tool_input_json::append_delta_by_index(
                            &mut pending_tools,
                            &pending_tool_indices,
                            index,
                            &partial_json,
                            |tool, delta| tool.input_json.push_str(delta),
                        ) {
                            tracing::warn!(
                                tool_block_index = index,
                                scope = "subagent",
                                "received tool input JSON delta without matching tool block"
                            );
                        }
                    }
                    StreamEvent::ContentBlockStop { .. } => {}
                    StreamEvent::Error {
                        error_type,
                        message,
                    } => {
                        let err = crate::agent::autocompact::classify_stream_error(
                            self.provider.name(),
                            &error_type,
                            &message,
                        );
                        if err.is_context_window_exceeded() && !reactive_overflow_retried {
                            reactive_overflow_retried = true;
                            let (outcome, compacted) =
                                crate::agent::autocompact::compact_json_messages_with_provider(
                                    self.provider.as_ref(),
                                    &self.model,
                                    &messages,
                                    crate::agent::CompactAction::Full,
                                    true,
                                )
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("reactive subagent compaction failed: {e}")
                                })?;
                            messages = compacted;
                            let after_current_tokens = match outcome {
                                crate::agent::autocompact::CompactionOutcome::Compacted {
                                    after_estimated_tokens,
                                    ..
                                } => after_estimated_tokens,
                                crate::agent::autocompact::CompactionOutcome::Skipped {
                                    ..
                                } => crate::agent::autocompact::estimate_messages_tokens(&messages),
                            };
                            last_known_context_tokens = 0;
                            auto_compact.on_success(after_current_tokens);
                            retry_after_compact = true;
                            break;
                        }
                        if crate::agent::autocompact::is_rate_limited_error(&err)
                            && !reactive_rate_limit_retried
                            && request_body_bytes >= large_retry_body_bytes
                        {
                            reactive_rate_limit_retried = true;
                            tracing::warn!(
                                compaction.reason = "rate_limit_large_request_stream",
                                trigger_body_bytes = request_body_bytes,
                                threshold_body_bytes = large_retry_body_bytes,
                                provider_family = telemetry.provider_family,
                                wire_shape = telemetry.wire_shape,
                                native_context_window = telemetry.native_context_window,
                                runtime_context_budget = telemetry.runtime_context_budget,
                                context_source = telemetry.context_source,
                                compaction_backend = telemetry.compaction_backend,
                                scope = "subagent",
                                force = true,
                                "rate-limited subagent stream is large; compacting before one retry"
                            );
                            let (outcome, compacted) =
                                crate::agent::autocompact::compact_json_messages_with_provider(
                                    self.provider.as_ref(),
                                    &self.model,
                                    &messages,
                                    crate::agent::CompactAction::Full,
                                    true,
                                )
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("rate-limit subagent compaction failed: {e}")
                                })?;
                            messages = compacted;
                            let after_current_tokens = match outcome {
                                crate::agent::autocompact::CompactionOutcome::Compacted {
                                    after_estimated_tokens,
                                    ..
                                } => after_estimated_tokens,
                                crate::agent::autocompact::CompactionOutcome::Skipped {
                                    ..
                                } => crate::agent::autocompact::estimate_messages_tokens(&messages),
                            };
                            last_known_context_tokens = 0;
                            auto_compact.on_success(after_current_tokens);
                            retry_after_compact = true;
                            break;
                        }
                        self.emit_activity_stream("error", message, None, true);
                        return Err(anyhow::Error::new(err));
                    }
                    StreamEvent::MessageStart { ref usage, .. } => {
                        // TASK-T3 (G4): accumulate Usage from message_start.
                        // Lock guard MUST NOT cross an .await — only sync work in here.
                        if let Some(ref t) = self.progress
                            && let Ok(mut g) = t.lock()
                        {
                            g.cumulative_input_tokens += usage.input_tokens;
                            g.cumulative_output_tokens += usage.output_tokens;
                            g.cumulative_cache_creation_tokens += usage.cache_creation_input_tokens;
                            g.cumulative_cache_read_tokens += usage.cache_read_input_tokens;
                            g.last_update = chrono::Utc::now();
                        }
                    }
                    StreamEvent::MessageDelta {
                        usage: Some(ref u), ..
                    } => {
                        if let Some(ref t) = self.progress
                            && let Ok(mut g) = t.lock()
                        {
                            g.cumulative_output_tokens += u.output_tokens;
                            g.last_update = chrono::Utc::now();
                        }
                    }
                    _ => {} // MessageDelta{usage:None}, MessageStop, Ping, etc.
                }
            }
            if retry_after_compact {
                continue;
            }
            reasoning_encrypted = turn_reasoning_encrypted;
            reactive_overflow_retried = false;
            reactive_rate_limit_retried = false;
            cumulative_billable_tokens += usage_acc.context_input_tokens;
            last_known_context_tokens = usage_acc.context_input_tokens;
            tracing::trace!(cumulative_billable_tokens, "subagent billable input tokens");

            // If no tool calls, subagent is done — return accumulated text
            if pending_tools.is_empty() {
                // Record final assistant text to transcript (AGT-024)
                if !text_content.is_empty() {
                    self.record_transcript(&serde_json::json!({
                        "role": "assistant",
                        "content": text_content,
                    }));
                }
                self.emit_activity_stream("final", "subagent turn complete", None, false);
                return Ok(text_content);
            }

            replay_tool_round(
                self,
                &mut messages,
                text_content,
                thinking_blocks,
                pending_tools,
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
