use super::*;

impl Agent {
    pub(in crate::agent) fn compaction_telemetry_for(
        &self,
        active_model: &str,
    ) -> CompactionTelemetry {
        compaction_telemetry(
            self.client.as_ref(),
            active_model,
            self.config
                .context
                .context_window_override
                .or_else(|| self.config.context.max_tokens.map(u64::from)),
            &self.config.working_dir,
        )
    }

    pub(in crate::agent) fn context_window_for(&self, active_model: &str) -> u64 {
        let resolved = archon_llm::context_window::resolve_context_window_for_work_dir(
            active_model,
            self.config
                .context
                .context_window_override
                .or_else(|| self.config.context.max_tokens.map(u64::from)),
            Some(self.client.as_ref()),
            Some(&self.config.working_dir),
        );
        resolved
            .runtime_context_budget
            .unwrap_or(resolved.context_window)
    }

    pub(in crate::agent) async fn force_reactive_compact(
        &mut self,
    ) -> Result<Vec<serde_json::Value>, AgentLoopError> {
        self.run_reactive_compaction(CompactAction::Full).await
    }

    async fn run_reactive_compaction(
        &mut self,
        action: CompactAction,
    ) -> Result<Vec<serde_json::Value>, AgentLoopError> {
        let active_model = self.active_model().await;
        let attribution = self.config.runtime_attribution_extra(
            "compaction",
            "reactive_request_projection",
            None,
            None,
            None,
        );
        match compact_json_messages_with_provider(
            self.client.as_ref(),
            &active_model,
            &self.state.messages,
            action,
            true,
            attribution,
        )
        .await
        {
            Ok((_, compacted)) => Ok(compacted),
            Err(error) => {
                self.state.auto_compact.on_failure(&error);
                Err(AgentLoopError::Compaction(error))
            }
        }
    }

    #[cfg(test)]
    pub(in crate::agent) async fn run_auto_compaction(
        &mut self,
        action: CompactAction,
        force: bool,
    ) -> Result<(), AgentLoopError> {
        self.state.auto_compact.compact_in_flight = true;
        let active_model = {
            let override_model = self.config.model_override.lock().await;
            if override_model.is_empty() {
                self.config.model.clone()
            } else {
                override_model.clone()
            }
        };
        let telemetry = self.compaction_telemetry_for(&active_model);
        tracing::info!(
            compaction.reason = "context_window_threshold",
            provider_family = telemetry.provider_family,
            wire_shape = telemetry.wire_shape,
            native_context_window = telemetry.native_context_window,
            runtime_context_budget = telemetry.runtime_context_budget,
            context_source = telemetry.context_source,
            compaction_backend = telemetry.compaction_backend,
            scope = "main_session",
            force,
            consecutive_failures = self.state.auto_compact.consecutive_failures,
            "auto-compaction attempt started"
        );
        // Try arithmetic before asking a model (#189 Phase 8). Much of what
        // fills a window — a file read three times unchanged, an error retried
        // successfully, a result whose full text is already spilled to disk —
        // is removable without judgement, and summarising it spends a request
        // and a wait to reach the same place.
        if self.prune_context_mechanically(&telemetry, force) {
            self.state.auto_compact.compact_in_flight = false;
            return Ok(());
        }
        let attribution = self.config.runtime_attribution_extra(
            "compaction",
            "auto_compaction",
            None,
            None,
            None,
        );
        let result = compact_json_messages_with_provider(
            self.client.as_ref(),
            &active_model,
            &self.state.messages,
            action,
            force,
            attribution,
        )
        .await;
        match result {
            Ok((
                CompactionOutcome::Compacted {
                    after_estimated_tokens,
                    ..
                },
                compacted,
            )) => {
                self.state.messages = compacted;
                self.state.last_known_context_tokens = 0;
                self.invalidate_memory_injector_cache();
                self.state.auto_compact.on_success(after_estimated_tokens);
                self.send_event(AgentEvent::CompactionTriggered).await;
                Ok(())
            }
            Ok((CompactionOutcome::Skipped { .. }, _)) => {
                self.state.auto_compact.on_cancel();
                Ok(())
            }
            Err(CompactionError::Cancelled) if !force => {
                self.state.auto_compact.on_cancel();
                tracing::debug!(
                    compaction.outcome = "cancelled",
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    actor = "main",
                    "proactive auto-compaction cancelled; continuing turn"
                );
                Ok(())
            }
            Err(err) if !force => {
                self.state.auto_compact.on_failure(&err);
                let consecutive_failures = self.state.auto_compact.consecutive_failures;
                tracing::warn!(
                    compaction.outcome = "auto_failed",
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    actor = "main",
                    consecutive_failures,
                    breaker_tripped = self.state.auto_compact.disabled,
                    error = %err,
                    "proactive auto-compaction failed; continuing turn"
                );
                Ok(())
            }
            Err(err) => {
                self.state.auto_compact.on_failure(&err);
                Err(AgentLoopError::Compaction(err))
            }
        }
    }
}
