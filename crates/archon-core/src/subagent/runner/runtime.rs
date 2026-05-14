use super::*;

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
        let mut reactive_overflow_retried = false;

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

            let window = archon_llm::context_window::resolve_context_window_for_work_dir(
                &self.model,
                self.agent_config
                    .context
                    .context_window_override
                    .or_else(|| self.agent_config.context.max_tokens.map(u64::from)),
                Some(self.provider.as_ref()),
                Some(&self.agent_config.working_dir),
            )
            .context_window;
            let effective_window =
                window.saturating_sub(self.agent_config.context.output_reserve_tokens);
            let threshold = (self.agent_config.context.compact_threshold
                - self.agent_config.context.preflight_safety_margin)
                .max(0.0);
            if let Some(action) = crate::agent::evaluate_compaction(
                crate::agent::autocompact::trigger_tokens(&messages),
                effective_window,
                &auto_compact,
                threshold,
            ) {
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
                        auto_compact.on_success(after_estimated_tokens);
                    }
                    Ok((crate::agent::autocompact::CompactionOutcome::Skipped { .. }, _)) => {
                        auto_compact.on_cancel();
                    }
                    Err(crate::agent::autocompact::CompactionError::Cancelled) => {
                        auto_compact.on_cancel();
                        tracing::debug!(
                            compaction.outcome = "cancelled", actor = %self.activity_actor_id
                                .as_deref().unwrap_or("subagent"), "proactive subagent compaction cancelled"
                        );
                    }
                    Err(e) => {
                        auto_compact.on_real_failure();
                        tracing::warn!(
                            compaction.outcome = "auto_failed", actor = %self.activity_actor_id
                                .as_deref().unwrap_or("subagent"), consecutive_failures =
                                auto_compact.consecutive_failures, breaker_tripped =
                                auto_compact.disabled, error = %e,
                            "proactive subagent compaction failed; continuing turn",
                        );
                    }
                }
            }

            let request = LlmRequest {
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
                reasoning_encrypted: None,
            };

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
            let mut pending_tools: Vec<PendingTool> = Vec::new();
            let mut current_tool_index: Option<u32> = None;
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
                            current_tool_index = Some(index);
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
                        }
                    }
                    StreamEvent::TextDelta { text, .. } => {
                        self.emit_activity_stream("text", text.clone(), None, false);
                        text_content.push_str(&text);
                    }
                    StreamEvent::ThinkingDelta { thinking, .. } => {
                        self.emit_activity_stream("thinking", thinking, None, false);
                    }
                    StreamEvent::InputJsonDelta {
                        index,
                        partial_json,
                    } => {
                        if Some(index) == current_tool_index
                            && let Some(tool) = pending_tools.last_mut()
                        {
                            tool.input_json.push_str(&partial_json);
                        }
                    }
                    StreamEvent::ContentBlockStop { .. } => {
                        current_tool_index = None;
                    }
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
                    _ => {} // SignatureDelta, MessageDelta{usage:None}, MessageStop, Ping, etc.
                }
            }
            if retry_after_compact {
                continue;
            }
            reactive_overflow_retried = false;
            cumulative_billable_tokens += usage_acc.context_input_tokens;
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

            // Build assistant message with text + tool_use blocks
            let mut assistant_content: Vec<serde_json::Value> = Vec::new();
            if !text_content.is_empty() {
                assistant_content.push(serde_json::json!({
                    "type": "text",
                    "text": text_content,
                }));
            }
            for tool in &pending_tools {
                let input: serde_json::Value =
                    serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));
                assistant_content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tool.id,
                    "name": tool.name,
                    "input": input,
                }));
            }
            let assistant_msg = serde_json::json!({
                "role": "assistant",
                "content": assistant_content,
            });
            self.record_transcript(&assistant_msg);
            messages.push(assistant_msg);

            // ── Three-phase parallel tool dispatch (v0.1.12) ──────────────
            // Mirrors claurst's proven pattern:
            //   Phase 1: sequential pre-hook pass (hooks/permissions are
            //            interactive and cannot be parallelized)
            //   Phase 2: concurrent execution via futures::future::join_all
            //            over Either<Left blocked, Right execute>.
            //            join_all preserves input order natively.
            //   Phase 3: assemble tool_result blocks in order, update
            //            progress tracker (sync, no .await across locks)
            //
            // Phase 1 — collect PreparedTool entries.
            struct PreparedTool {
                id: String,
                name: String,
                input: serde_json::Value,
            }
            let mut prepared: Vec<PreparedTool> = Vec::with_capacity(pending_tools.len());
            for tool in &pending_tools {
                let input: serde_json::Value =
                    serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));
                prepared.push(PreparedTool {
                    id: tool.id.clone(),
                    name: tool.name.clone(),
                    input,
                });
            }

            // Phase 2 — execute all tools concurrently via join_all.
            // Each async block owns its cloned name/input/registry.
            let registry = Arc::clone(&self.registry);
            let exec_futures: Vec<_> = prepared
                .iter()
                .map(|p| {
                    let name = p.name.clone();
                    let input = p.input.clone();
                    let registry = Arc::clone(&registry);
                    let ctx = self.tool_context.clone();
                    async move { registry.dispatch(&name, input, &ctx).await }
                })
                .collect();

            let exec_results: Vec<ToolResult> = join_all(exec_futures).await;

            // Phase 3 — assemble tool_result blocks IN ORDER.
            // join_all preserves input order, so zip is correct.
            let mut tool_results: Vec<serde_json::Value> = Vec::with_capacity(prepared.len());
            for (p, result) in prepared.iter().zip(exec_results.into_iter()) {
                // Progress update — sync only, lock never crosses .await
                if let Some(ref t) = self.progress
                    && let Ok(mut g) = t.lock()
                {
                    g.tool_use_count += 1;
                    if g.recent_activities.len() >= 5 {
                        g.recent_activities.pop_front();
                    }
                    g.recent_activities
                        .push_back(crate::subagent::ToolActivity {
                            tool_name: p.name.clone(),
                            timestamp: chrono::Utc::now(),
                        });
                    g.last_update = chrono::Utc::now();
                }
                tool_results.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": p.id,
                    "content": result.content,
                    "is_error": result.is_error,
                }));
                self.emit_activity_stream(
                    "tool_result",
                    summarize_tool_output(&result.content),
                    Some(&p.name),
                    result.is_error,
                );
            }

            // Add tool results as a user message
            let tool_result_msg = serde_json::json!({
                "role": "user",
                "content": tool_results,
            });
            self.record_transcript(&tool_result_msg);
            messages.push(tool_result_msg);

            // AGT-026: Drain pending messages at tool round boundary and inject as user turns
            let pending = self.drain_pending_as_user_turns().await;
            for msg in pending {
                self.record_transcript(&msg);
                messages.push(msg);
            }
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
