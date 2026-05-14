use super::*;

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
    pub async fn compact(&mut self, subcommand: Option<&str>) -> String {
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
            return "Nothing to compact (fewer than 5 messages).".into();
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
            Some("auto") | None => {
                let active_model = self.active_model_for_compaction().await;
                let context_window = self.context_window_for(&active_model);
                let usage_ratio = before_tokens as f32 / context_window as f32;
                select_strategy(usage_ratio)
            }
            Some(other) => {
                return format!(
                    "Unknown /compact subcommand: '{other}'. Use auto, micro, or snip."
                );
            }
        };

        // If select_strategy says no compaction needed and user didn't force a strategy
        let effective_strategy = match effective_strategy {
            Some(s) => s,
            None => {
                return "Context usage is below 60 %; no compaction needed.".into();
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
                    return "Too few turns to snip.".into();
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
                    Err(e) => return format!("Snip failed: {e}"),
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
                        Err(err) => return format!("Compaction failed: {err}"),
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
                            return output.message;
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
        format!(
            "Compacted conversation ({strategy_label}): {before_k:.1}k → {after_k:.1}k tokens ({removed_k:.1}k removed, {message_count} messages)"
        )
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
mod tests {
    use super::*;
    use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
    use std::sync::Mutex;

    struct FailingSummaryProvider;

    #[async_trait::async_trait]
    impl LlmProvider for FailingSummaryProvider {
        fn name(&self) -> &str {
            "failing-summary"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![]
        }

        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            Err(LlmError::RateLimited {
                retry_after_secs: 30,
            })
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            unreachable!("manual compaction uses streaming summaries")
        }

        fn supports_feature(&self, _feature: ProviderFeature) -> bool {
            false
        }
    }

    struct CapturingSummaryProvider {
        captured: Arc<Mutex<Option<LlmRequest>>>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for CapturingSummaryProvider {
        fn name(&self) -> &str {
            "capturing-summary"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![]
        }

        async fn stream(
            &self,
            request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            *self.captured.lock().expect("capture lock") = Some(request);
            let (tx, rx) = tokio::sync::mpsc::channel(2);
            tx.send(StreamEvent::TextDelta {
                index: 0,
                text: "Manual path summary.".into(),
            })
            .await
            .expect("send summary text");
            tx.send(StreamEvent::MessageStop)
                .await
                .expect("send message stop");
            Ok(rx)
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            unreachable!("manual compaction uses streaming summaries")
        }

        fn supports_feature(&self, _feature: ProviderFeature) -> bool {
            false
        }
    }

    fn test_agent_with_provider(provider: Arc<dyn LlmProvider>) -> Agent {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Agent::new(
            provider,
            ToolRegistry::new(),
            AgentConfig::default(),
            tx,
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(
                &std::env::temp_dir(),
            ))),
        )
    }

    fn serialized_request_len(request: &LlmRequest) -> usize {
        serde_json::to_vec(&serde_json::json!({
            "model": &request.model,
            "max_tokens": request.max_tokens,
            "system": &request.system,
            "messages": &request.messages,
            "tools": &request.tools,
            "thinking": &request.thinking,
            "speed": &request.speed,
            "effort": &request.effort,
            "extra": &request.extra,
            "request_origin": &request.request_origin,
            "reasoning_encrypted": &request.reasoning_encrypted,
        }))
        .expect("serialize request envelope")
        .len()
    }

    fn test_agent() -> Agent {
        test_agent_with_provider(Arc::new(FailingSummaryProvider))
    }

    #[tokio::test]
    async fn manual_compact_reports_summary_failure_without_synthetic_fallback() {
        let mut agent = test_agent();
        agent.state.messages = (0..6)
            .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
            .collect();

        let status = agent.compact(Some("micro")).await;

        assert!(status.contains("Compaction failed: provider summary failed"));
        assert!(status.contains("rate limited: retry after 30s"));
        assert!(!status.contains("Compacted conversation"));
        assert_eq!(agent.state.messages.len(), 6);
    }

    #[tokio::test]
    async fn manual_compact_path_pre_trims_huge_history() {
        let captured = Arc::new(Mutex::new(None));
        let provider = CapturingSummaryProvider {
            captured: Arc::clone(&captured),
        };
        let mut agent = test_agent_with_provider(Arc::new(provider));
        agent.state.messages = (0..200)
            .map(|i| {
                serde_json::json!({
                    "role": if i % 2 == 0 { "user" } else { "assistant" },
                    "content": "x".repeat(10_000),
                })
            })
            .collect();

        let status = agent.compact(Some("micro")).await;

        assert!(status.contains("Compacted conversation"));
        let request = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("manual compact should call provider");
        let body_len = serialized_request_len(&request);
        assert!(
            body_len <= 640_000,
            "manual compact body should be bounded; got {}",
            body_len
        );
    }
}
