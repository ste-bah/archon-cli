use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualCompactOutcome {
    Compacted {
        status: String,
        messages_before: usize,
        messages_after: usize,
        before_tokens: u64,
        after_tokens: u64,
    },
    BelowThreshold { status: String },
    Skipped { status: String },
    Failed { status: String },
}

impl ManualCompactOutcome {
    pub fn status(&self) -> &str {
        match self {
            Self::Compacted { status, .. }
            | Self::BelowThreshold { status }
            | Self::Skipped { status }
            | Self::Failed { status } => status,
        }
    }

    pub fn into_status(self) -> String {
        match self {
            Self::Compacted { status, .. }
            | Self::BelowThreshold { status }
            | Self::Skipped { status }
            | Self::Failed { status } => status,
        }
    }
}

impl Agent {
    /// Clear conversation history, keeping config and subsystems intact.
    pub async fn clear_conversation(&mut self) {
        self.state.messages.clear();
        self.state.total_input_tokens = 0;
        self.state.total_output_tokens = 0;
        self.state.last_known_context_tokens = 0;
        self.turn_number = 0;
        self.memory_injector.invalidate_cache();
        // Reset shared session stats so /status and /cost reflect the cleared state
        {
            let mut stats = self.session_stats.lock().await;
            *stats = SessionStats::default();
        }
    }

    /// Clear conversation history without holding `&mut self` across the
    /// returned future. Performs the synchronous `&mut self` work
    /// up-front and returns an owned `Send + 'static` future that
    /// completes the async work (resetting shared `session_stats`).
    ///
    /// Call sites drop the `MutexGuard<Agent>` before `.await`ing the
    /// returned future, so the guard is not held across await. Needed
    /// inside `tokio::spawn` blocks where rustc's HRTB inference
    /// otherwise rejects the spawn's `Send` bound.
    ///
    /// Semantically identical to [`Agent::clear_conversation`].
    pub fn clear_conversation_detached(
        &mut self,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.state.messages.clear();
        self.state.total_input_tokens = 0;
        self.state.total_output_tokens = 0;
        self.state.last_known_context_tokens = 0;
        self.turn_number = 0;
        self.memory_injector.invalidate_cache();
        let stats = self.session_stats.clone();
        async move {
            let mut guard = stats.lock().await;
            *guard = SessionStats::default();
        }
    }

    /// GAP 1: Trigger conversation compaction.
    ///
    /// Converts the agent's messages to ContextMessages, runs compaction,
    /// and replaces the conversation state. Fires PreCompact and PostCompact
    /// hooks around the compaction. Returns a human-readable status message.
    ///
    /// `subcommand` selects the strategy:
    /// - `None` or `Some("auto")` — pick strategy automatically via `select_strategy`
    /// - `Some("micro")` — microcompact (summarize oldest 30 %)
    /// - `Some("snip")` — snip oldest turns without summarization
    pub async fn compact(&mut self, subcommand: Option<&str>) -> ManualCompactOutcome {
        use crate::commands::handle_compact;
        use archon_context::compact::select_strategy;
        use archon_context::messages::ContextMessage;
        use archon_context::microcompact::microcompact_messages;
        use archon_context::snip::snip_messages;

        // Convert JSON messages to ContextMessages
        let context_msgs: Vec<ContextMessage> = self
            .state
            .messages
            .iter()
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user").to_string();
                let content = m["content"].clone();
                let text_len = match &content {
                    serde_json::Value::String(s) => s.len(),
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .map(|v| {
                            v.get("text")
                                .and_then(|t| t.as_str())
                                .map_or(0, |s| s.len())
                        })
                        .sum(),
                    _ => 0,
                };
                ContextMessage {
                    role,
                    content,
                    estimated_tokens: (text_len as f64 / 4.0).ceil() as u64,
                }
            })
            .collect();

        if context_msgs.len() < 5 {
            return ManualCompactOutcome::Skipped {
                status: "Nothing to compact (fewer than 5 messages).".into(),
            };
        }

        let message_count = context_msgs.len();
        let before_tokens: u64 = if self.state.last_known_context_tokens > 0 {
            self.state.last_known_context_tokens
        } else {
            context_msgs.iter().map(|m| m.estimated_tokens).sum()
        };

        // Resolve the effective strategy.
        // "auto" (or no subcommand) uses select_strategy based on context usage ratio.
        let effective_strategy = match subcommand {
            Some("micro") => Some(archon_context::boundary::CompactionStrategy::Micro),
            Some("snip") => Some(archon_context::boundary::CompactionStrategy::Snip),
            Some("force") => Some(match self.config.context.manual_compact_force_strategy.as_str()
            {
                "snip" => archon_context::boundary::CompactionStrategy::Snip,
                _ => archon_context::boundary::CompactionStrategy::Micro,
            }),
            Some("auto") | None => {
                let active_model = self.active_model_for_compaction().await;
                let context_window = self.context_window_for(&active_model);
                let usage_ratio = before_tokens as f32 / context_window as f32;
                select_strategy(usage_ratio)
            }
            Some(other) => {
                return ManualCompactOutcome::Failed {
                    status: format!(
                        "Unknown /compact subcommand: '{other}'. Use auto, force, micro, or snip."
                    ),
                };
            }
        };

        // If select_strategy says no compaction needed and user didn't force a strategy
        let effective_strategy = match effective_strategy {
            Some(s) => s,
            None => {
                return ManualCompactOutcome::BelowThreshold {
                    status: "Context usage is below 60 %; no compaction needed.".into(),
                };
            }
        };

        // Fire PreCompact hook
        if let Some(ref registry) = self.hook_registry {
            let payload = serde_json::json!({
                "hook_event": "PreCompact",
                "message_count": message_count,
                "token_count": before_tokens,
                "strategy": effective_strategy.to_string(),
            });
            registry
                .execute_hooks(
                    crate::hooks::HookEvent::PreCompact,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await;
        }

        // Dispatch based on the resolved strategy.
        let (result_messages, strategy_label, _status_message) = match effective_strategy {
            archon_context::boundary::CompactionStrategy::Snip => {
                // Snip: remove oldest turns without LLM summarization.
                let total_turns = archon_context::snip::count_turns(&context_msgs);
                if total_turns < 3 {
                    return ManualCompactOutcome::Skipped {
                        status: "Too few turns to snip.".into(),
                    };
                }
                // Snip the oldest ~50 % of turns (at least 1).
                let snip_end = (total_turns / 2).max(1);
                match snip_messages(&context_msgs, 1, snip_end) {
                    Ok((msgs, boundary)) => {
                        let label = "snip";
                        let status = format!(
                            "Snipped turns 1–{snip_end} ({} tokens removed)",
                            boundary.tokens_removed
                        );
                        (msgs, label, status)
                    }
                    Err(e) => {
                        return ManualCompactOutcome::Failed {
                            status: format!("Snip failed: {e}"),
                        };
                    }
                }
            }

            archon_context::boundary::CompactionStrategy::Micro
            | archon_context::boundary::CompactionStrategy::Auto => {
                // Both Micro and Auto need an LLM-generated summary.
                let active_model = self.active_model_for_compaction().await;
                let mut summary_text =
                    match super::autocompact::generate_compaction_summary_structured(
                        self.client.as_ref(),
                        &active_model,
                        &self.state.messages,
                    )
                    .await
                    {
                        Ok(summary) => summary,
                        Err(err) => {
                            return ManualCompactOutcome::Failed {
                                status: format!("Compaction failed: {err}"),
                            };
                        }
                    };

                // Wire 4: Inject active plan context into compaction summary.
                if let Some(ref plan_store) = self.plan_store
                    && let Some(plan_ctx) = archon_session::plan::plan_context_for_compaction(
                        plan_store,
                        &self.config.session_id,
                    )
                {
                    summary_text.push_str(&plan_ctx);
                }

                match effective_strategy {
                    archon_context::boundary::CompactionStrategy::Micro => {
                        let preserve = archon_context::compact::DEFAULT_PRESERVE_RECENT_TURNS;
                        let (msgs, boundary) =
                            microcompact_messages(&context_msgs, &summary_text, preserve);
                        let label = "micro";
                        let status =
                            format!("Microcompacted: {} tokens removed", boundary.tokens_removed);
                        (msgs, label, status)
                    }
                    _ => {
                        // Auto / default: full compaction via handle_compact
                        let output = handle_compact(&context_msgs, &summary_text);
                        let label = "auto";
                        let status = output.message.clone();
                        if output.mutated {
                            (output.messages, label, status)
                        } else {
                            return ManualCompactOutcome::Skipped {
                                status: output.message,
                            };
                        }
                    }
                }
            }
        };

        // Replace the conversation messages with the compacted version
        self.state.messages = result_messages
            .iter()
            .map(|cm| {
                serde_json::json!({
                    "role": cm.role,
                    "content": cm.content,
                })
            })
            .collect();
        // Reset stale API context telemetry — matches the fix in autocompact.rs:202.
        // The next API response will repopulate last_known_context_tokens authoritatively.
        self.state.last_known_context_tokens = 0;
        // Invalidate memory cache since context changed
        self.memory_injector.invalidate_cache();

        // CRIT-15 (ITEM 5): Snapshot inner voice state on compaction and persist to memory graph.
        if let Some(ref iv) = self.inner_voice {
            let snapshot = iv.lock().await.on_compaction();
            tracing::debug!(
                "inner voice snapshot on compaction: confidence={:.2}, energy={:.2}, turns={}",
                snapshot.confidence,
                snapshot.energy,
                snapshot.turn_count
            );
            // Persist snapshot so it can be restored via InnerVoice::from_snapshot on resume.
            if let Some(ref graph) = self.memory
                && let Ok(json) = serde_json::to_string(&snapshot)
            {
                let _ = graph.store_memory(
                    &json,
                    "inner_voice_snapshot",
                    archon_memory::types::MemoryType::Fact,
                    90.0,
                    &["inner_voice_snapshot".to_string()],
                    "agent",
                    "",
                );
            }
        }

        // Compute post-compaction token count
        let after_tokens: u64 = result_messages.iter().map(|m| m.estimated_tokens).sum();
        let tokens_removed = before_tokens.saturating_sub(after_tokens);
        let outcome = super::autocompact::CompactionOutcome::Compacted {
            before_tokens,
            after_estimated_tokens: after_tokens,
            messages_before: message_count,
            messages_after: result_messages.len(),
        };
        let outcome_json = serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null);

        // Fire PostCompact hook
        if let Some(ref registry) = self.hook_registry {
            let payload = serde_json::json!({
                "hook_event": "PostCompact",
                "strategy": strategy_label,
                "tokens_removed": tokens_removed,
                "tokens_remaining": after_tokens,
                "outcome": outcome_json,
            });
            registry
                .execute_hooks(
                    crate::hooks::HookEvent::PostCompact,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await;
        }

        // Return detailed summary
        let before_k = before_tokens as f64 / 1000.0;
        let after_k = after_tokens as f64 / 1000.0;
        let removed_k = tokens_removed as f64 / 1000.0;
        ManualCompactOutcome::Compacted {
            status: format!(
                "Compacted conversation ({strategy_label}): {before_k:.1}k → {after_k:.1}k tokens ({removed_k:.1}k removed, {message_count} messages)"
            ),
            messages_before: message_count,
            messages_after: result_messages.len(),
            before_tokens,
            after_tokens,
        }
    }

    async fn active_model_for_compaction(&self) -> String {
        let override_model = self.config.model_override.lock().await;
        if override_model.is_empty() {
            self.config.model.clone()
        } else {
            override_model.clone()
        }
    }
}

#[cfg(test)]
#[path = "compaction_tests.rs"]
mod tests;
