use std::sync::Arc;

use archon_session::storage::{CompactionSummaryStatus, CompactionTelemetryRecord, SessionStore};

use super::*;

impl Agent {
    pub fn set_session_store(&mut self, store: Arc<SessionStore>) {
        self.session_store = Some(Arc::clone(&store));
        let recoverable = match store.recoverable_compaction_segments(&self.config.session_id) {
            Ok(segments) => segments,
            Err(error) => {
                tracing::warn!(%error, "failed to recover compaction summaries");
                return;
            }
        };
        let active_model = self.config.model.clone();
        for segment in recoverable {
            let body = match store.load_compaction_segment_body(&segment.id) {
                Ok(body) => body,
                Err(error) => {
                    tracing::warn!(%error, segment_id = %segment.id, "failed to load recoverable segment");
                    continue;
                }
            };
            let source = match body
                .iter()
                .map(|message| serde_json::from_str(message))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(source) => source,
                Err(error) => {
                    let failure = format!("malformed persisted source: {error}");
                    if let Err(mark_error) =
                        store.mark_compaction_segment_source_invalid(&segment.id, &failure)
                    {
                        tracing::warn!(%mark_error, segment_id = %segment.id, "failed to mark malformed segment source");
                    }
                    continue;
                }
            };
            if let Err(error) = autocompact::validate_compaction_source(&source) {
                let failure = format!("invalid persisted source message: {error}");
                if let Err(mark_error) =
                    store.mark_compaction_segment_source_invalid(&segment.id, &failure)
                {
                    tracing::warn!(%mark_error, segment_id = %segment.id, "failed to mark invalid segment source");
                }
                continue;
            }
            self.spawn_segment_summary(Arc::clone(&store), segment, source, &active_model);
        }
    }

    pub fn recall_compaction_segment(
        &self,
        segment_id: &str,
        limit_bytes: usize,
    ) -> Result<String, AgentLoopError> {
        let store = self.session_store.as_ref().ok_or_else(|| {
            AgentLoopError::ApiError("compaction session store unavailable".into())
        })?;
        let body = store
            .load_authorized_compaction_segment_body(&self.config.session_id, segment_id)
            .map_err(session_error)?;
        let source = body
            .iter()
            .map(|message| serde_json::from_str(message))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                AgentLoopError::ApiError(format!("malformed persisted source: {error}"))
            })?;
        autocompact::validate_compaction_source(&source).map_err(|error| {
            AgentLoopError::ApiError(format!("invalid persisted source message: {error}"))
        })?;
        Ok(autocompact::bound_recalled_segment(
            segment_id,
            &body,
            limit_bytes.min(self.state.max_tool_result_bytes),
        ))
    }

    pub async fn flush_compaction_summaries(&mut self, timeout: std::time::Duration) -> usize {
        let mut handles = std::mem::take(&mut self.compaction_summary_tasks);
        let pending = handles.len();
        let deadline = std::time::Instant::now() + timeout;
        for mut handle in handles.drain(..) {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                handle.abort();
                let _ = handle.await;
                continue;
            }
            if tokio::time::timeout(remaining, &mut handle).await.is_err() {
                handle.abort();
                let _ = handle.await;
            }
        }
        pending
    }

    pub(super) fn messages_for_turn_request(
        &self,
        active_model: &str,
    ) -> Result<Vec<serde_json::Value>, AgentLoopError> {
        let messages = if self.staged_compaction_due(active_model) {
            self.assemble_stored_compaction_messages()?
        } else {
            self.state.messages.clone()
        };
        Ok(tool_result_context::project_messages_for_request(
            &messages,
            self.config.context.preserve_recent_turns,
        ))
    }

    pub(super) fn close_completed_compaction_segment(&mut self, active_model: &str) {
        let Some(store) = self.session_store.as_ref().map(Arc::clone) else {
            return;
        };
        let segments = match store.list_compaction_segments(&self.config.session_id) {
            Ok(segments) => segments,
            Err(error) => {
                tracing::warn!(%error, "failed to list compaction segments");
                return;
            }
        };
        let start = segments
            .last()
            .map_or(0, |segment| segment.end_index as usize + 1);
        let preserve = self.preserve_recent_message_count();
        let Some(span) =
            autocompact::next_closed_segment_span(&self.state.messages, start, preserve)
        else {
            return;
        };
        let source = &self.state.messages[span.start..=span.end];
        if let Err(error) = autocompact::validate_compaction_source(source) {
            tracing::warn!(%error, "refusing to close invalid compaction source");
            return;
        }
        let body: Vec<String> = source.iter().map(serde_json::Value::to_string).collect();
        let segment_id = format!(
            "segment:{}:{}:{}",
            self.config.session_id, span.start, span.end
        );
        let ledger = autocompact::derive_segment_ledger(
            &self.config.session_id,
            span.start,
            source,
            self.state.max_tool_result_bytes,
        );
        let telemetry = CompactionTelemetryRecord {
            id: format!("telemetry:{segment_id}:closed"),
            session_id: self.config.session_id.clone(),
            action: "segment_closed".into(),
            payload: serde_json::json!({"segment_id": segment_id}).to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let segment = match store.close_compaction_segment_with_records(
            &self.config.session_id,
            span.start as u64,
            span.end as u64,
            &body,
            &ledger,
            Some(&telemetry),
        ) {
            Ok(segment) => segment,
            Err(error) => {
                tracing::warn!(%error, "failed to close compaction segment");
                return;
            }
        };
        self.spawn_segment_summary(store, segment, source.to_vec(), active_model);
    }

    fn staged_compaction_due(&self, active_model: &str) -> bool {
        let window = self
            .context_window_for(active_model)
            .saturating_sub(self.config.context.output_reserve_tokens);
        let tokens = self
            .state
            .last_known_context_tokens
            .max(autocompact::trigger_tokens(&self.state.messages));
        let threshold = (self.config.context.compact_threshold
            - self.config.context.preflight_safety_margin)
            .max(0.0);
        let due =
            autocompact::evaluate_compaction(tokens, window, &self.state.auto_compact, threshold)
                .is_some();
        if due {
            self.log_context_attribution(tokens, window);
        }
        due
    }

    /// Record which messages are filling the window, at the moment it matters.
    ///
    /// Compaction has always known *that* the window is 82% full and never
    /// *which* messages account for it, so the only diagnosis available after
    /// the fact was "it compacted". #189 Phase 3 makes the surface answerable;
    /// this is where the answer is worth having.
    fn log_context_attribution(&self, tokens: u64, window: u64) {
        let surface = self.state.token_surface();
        let top = surface.top_contributors(3);
        if top.is_empty() {
            return;
        }
        let heaviest: Vec<String> = top
            .iter()
            .map(|node| format!("#{}={}", node.message_index, node.estimated_tokens))
            .collect();
        tracing::info!(
            compaction.trigger_tokens = tokens,
            compaction.window = window,
            compaction.attributed_total = surface.total(),
            compaction.calibrated = surface.calibration().is_calibrated(),
            compaction.heaviest_messages = heaviest.join(" "),
            "context attribution at compaction"
        );
    }

    fn assemble_stored_compaction_messages(
        &self,
    ) -> Result<Vec<serde_json::Value>, AgentLoopError> {
        let Some(store) = &self.session_store else {
            return Ok(self.state.messages.clone());
        };
        let segments = store
            .list_compaction_segments(&self.config.session_id)
            .map_err(session_error)?;
        let ledger = store
            .list_compaction_ledger_records(&self.config.session_id)
            .map_err(session_error)?;
        let assembled = autocompact::assemble_compacted_messages(
            &self.state.messages,
            &segments,
            &ledger,
            self.preserve_recent_message_count(),
            self.state.max_tool_result_bytes,
            |_| {},
        );
        let record = CompactionTelemetryRecord {
            id: format!(
                "telemetry:{}:assembly:{}",
                self.config.session_id, self.turn_number
            ),
            session_id: self.config.session_id.clone(),
            action: "threshold_assembly".into(),
            payload: serde_json::json!({
                "before_bytes": serde_json::to_vec(&self.state.messages).map_or(0, |body| body.len()),
                "after_bytes": serde_json::to_vec(&assembled.messages).map_or(0, |body| body.len()),
                "swapped_segment_ids": assembled.swapped_segment_ids,
                "digest_fallback_count": assembled.digest_fallback_count,
            })
            .to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        store
            .put_compaction_telemetry(&record)
            .map_err(session_error)?;
        Ok(assembled.messages)
    }

    fn preserve_recent_message_count(&self) -> usize {
        let turns = self.config.context.preserve_recent_turns as usize;
        if turns == 0 {
            return 0;
        }
        let mut seen = 0;
        for (index, message) in self.state.messages.iter().enumerate().rev() {
            if is_user_prompt(message) {
                seen += 1;
                if seen == turns {
                    return self.state.messages.len() - index;
                }
            }
        }
        self.state.messages.len()
    }

    fn spawn_segment_summary(
        &mut self,
        store: Arc<SessionStore>,
        segment: archon_session::storage::CompactionSegment,
        source: Vec<serde_json::Value>,
        active_model: &str,
    ) {
        if segment.summary_status == CompactionSummaryStatus::Succeeded {
            return;
        }
        let resolution = self.resolve_segment_summary_model(active_model);
        let model = resolution.model.clone();
        let attribution = self.config.runtime_attribution_extra(
            "compaction",
            "background_segment_summary",
            None,
            None,
            None,
        );
        let attribution = merge_summary_resolution(attribution, &resolution, self.client.name());
        let attribution_text = attribution.to_string();
        let Some(claim) = store
            .claim_compaction_segment_summary(&segment.id, &model, &attribution_text)
            .ok()
            .flatten()
        else {
            return;
        };
        let client = Arc::clone(&self.client);
        let session_id = self.config.session_id.clone();
        self.compaction_summary_tasks.push(tokio::spawn(async move {
            let started = std::time::Instant::now();
            let result = autocompact::generate_segment_summary_with_usage(
                client.as_ref(),
                &model,
                &source,
                attribution,
            )
            .await;
            match result {
                Ok(summary) => {
                    let cost = crate::cost::estimate_turn_cost_usd(
                        &model,
                        summary.input_tokens,
                        summary.output_tokens,
                        0,
                        0,
                    );
                    let completed = store
                        .complete_compaction_segment_summary(
                            &segment.id,
                            &claim,
                            &summary.text,
                            summary.input_tokens,
                            summary.output_tokens,
                            cost,
                        )
                        .unwrap_or(false);
                    if !completed {
                        return;
                    }
                    let record = CompactionTelemetryRecord {
                        id: format!("telemetry:{}:summary-completed", segment.id),
                        session_id,
                        action: "summary_completed".into(),
                        payload: serde_json::json!({
                            "segment_id": segment.id,
                            "model": model,
                            "input_tokens": summary.input_tokens,
                            "output_tokens": summary.output_tokens,
                            "cost": cost,
                            "duration_ms": started.elapsed().as_millis(),
                        })
                        .to_string(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = store.put_compaction_telemetry(&record);
                }
                Err(error) => {
                    let failure = match error {
                        autocompact::CompactionError::Provider(error) => {
                            format!("provider summary failed: {error}")
                        }
                        autocompact::CompactionError::Cancelled => {
                            "summary cancelled: compaction summary was cancelled".to_string()
                        }
                        error => error.to_string(),
                    };
                    let _ = store.fail_compaction_segment_summary(&segment.id, &claim, &failure);
                }
            }
        }));
    }

    fn resolve_segment_summary_model(
        &self,
        active_model: &str,
    ) -> autocompact::ResolvedCompactionModel {
        let models = self.client.models();
        let available: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        autocompact::resolve_compaction_model(
            self.config.context.compaction_model.as_deref(),
            None,
            active_model,
            &available,
        )
    }
}

fn merge_summary_resolution(
    mut attribution: serde_json::Value,
    resolution: &autocompact::ResolvedCompactionModel,
    provider: &str,
) -> serde_json::Value {
    attribution["archon_runtime"]["summary_provider"] = serde_json::json!(provider);
    attribution["archon_runtime"]["summary_model_source"] =
        serde_json::json!(match resolution.source {
            autocompact::CompactionModelSource::Explicit => "explicit",
            autocompact::CompactionModelSource::ProviderPolicy => "provider_policy",
            autocompact::CompactionModelSource::ActiveFallback => "active_fallback",
        });
    attribution["archon_runtime"]["summary_model_fallback_reason"] =
        serde_json::json!(resolution.fallback_reason);
    attribution
}

fn is_user_prompt(message: &serde_json::Value) -> bool {
    if message.get("role").and_then(serde_json::Value::as_str) != Some("user") {
        return false;
    }
    !message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|blocks| {
            !blocks.is_empty()
                && blocks.iter().all(|block| {
                    block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
                })
        })
}

fn session_error(error: archon_session::storage::SessionError) -> AgentLoopError {
    AgentLoopError::ApiError(format!("compaction session storage failed: {error}"))
}
