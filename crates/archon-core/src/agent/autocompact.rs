use super::*;

const MICRO_COMPACT_FRACTION: f32 = 0.65;
const MAX_COMPACT_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactAction {
    Micro,
    Full,
}

#[derive(Debug, Clone, Default)]
pub struct AutoCompactState {
    pub compaction_count: u32,
    pub consecutive_failures: u32,
    pub disabled: bool,
    pub compact_in_flight: bool,
    pub last_compact_at_tokens: u64,
}

impl AutoCompactState {
    pub fn should_attempt(&self) -> bool {
        !self.disabled && !self.compact_in_flight
    }

    pub fn on_success(&mut self, tokens: u64) {
        self.compaction_count += 1;
        self.consecutive_failures = 0;
        self.compact_in_flight = false;
        self.last_compact_at_tokens = tokens;
    }

    pub fn on_real_failure(&mut self) {
        self.consecutive_failures += 1;
        self.compact_in_flight = false;
        if self.consecutive_failures >= MAX_COMPACT_FAILURES {
            self.disabled = true;
        }
    }

    pub fn on_cancel(&mut self) {
        self.compact_in_flight = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    Compacted {
        before_tokens: u64,
        after_estimated_tokens: u64,
        messages_before: usize,
        messages_after: usize,
    },
    Skipped {
        reason: SkipReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    BelowThreshold,
    NoSafeBoundary,
    Disabled,
    InFlight,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CompactionError {
    #[error("no safe compaction boundary")]
    NoSafeBoundary,
    #[error("provider summary failed: {0}")]
    Provider(#[from] archon_llm::provider::LlmError),
    #[error("compaction summary was cancelled")]
    Cancelled,
    #[error("invalid compaction summary: {0}")]
    InvalidSummary(String),
}

pub fn evaluate_compaction(
    tokens_used: u64,
    context_window: u64,
    state: &AutoCompactState,
    threshold: f32,
) -> Option<CompactAction> {
    if context_window == 0 || !state.should_attempt() {
        return None;
    }
    let fraction = tokens_used as f32 / context_window as f32;
    if fraction >= threshold {
        Some(CompactAction::Full)
    } else if fraction >= MICRO_COMPACT_FRACTION {
        Some(CompactAction::Micro)
    } else {
        None
    }
}

pub fn estimate_message_tokens(message: &serde_json::Value) -> u64 {
    (message.to_string().len() as f64 / 4.0).ceil() as u64
}

pub fn estimate_messages_tokens(messages: &[serde_json::Value]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

pub(crate) fn trigger_tokens(messages: &[serde_json::Value]) -> u64 {
    estimate_messages_tokens(messages)
}

pub fn classify_stream_error(
    provider: &str,
    error_type: &str,
    message: &str,
) -> archon_llm::provider::LlmError {
    archon_llm::context_window::classify_context_window_error(
        None,
        Some(error_type),
        None,
        message,
        Some(provider),
        None,
    )
    .unwrap_or_else(|| archon_llm::provider::LlmError::Http(format!("{error_type}: {message}")))
}

impl Agent {
    pub(super) fn context_window_for(&self, active_model: &str) -> u64 {
        archon_llm::context_window::resolve_context_window_for_work_dir(
            active_model,
            self.config
                .context
                .context_window_override
                .or_else(|| self.config.context.max_tokens.map(u64::from)),
            Some(self.client.as_ref()),
            Some(&self.config.working_dir),
        )
        .context_window
    }

    pub(super) async fn maybe_auto_compact(
        &mut self,
        active_model: &str,
    ) -> Result<(), AgentLoopError> {
        let window = self.context_window_for(active_model);
        let tokens = if self.state.last_known_context_tokens > 0 {
            self.state.last_known_context_tokens
        } else {
            trigger_tokens(&self.state.messages)
        };
        let effective_window = window.saturating_sub(self.config.context.output_reserve_tokens);
        let threshold = (self.config.context.compact_threshold
            - self.config.context.preflight_safety_margin)
            .max(0.0);
        let Some(action) = evaluate_compaction(
            tokens,
            effective_window,
            &self.state.auto_compact,
            threshold,
        ) else {
            return Ok(());
        };
        self.run_auto_compaction(action, false).await
    }

    pub(super) async fn force_reactive_compact(&mut self) -> Result<(), AgentLoopError> {
        self.run_auto_compaction(CompactAction::Full, true).await
    }

    async fn run_auto_compaction(
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
        let result = compact_json_messages_with_provider(
            self.client.as_ref(),
            &active_model,
            &self.state.messages,
            action,
            force,
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
                self.memory_injector.invalidate_cache();
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
                    actor = "main",
                    "proactive auto-compaction cancelled; continuing turn"
                );
                Ok(())
            }
            Err(err) if !force => {
                self.state.auto_compact.on_real_failure();
                let consecutive_failures = self.state.auto_compact.consecutive_failures;
                tracing::warn!(
                    compaction.outcome = "auto_failed",
                    actor = "main",
                    consecutive_failures,
                    breaker_tripped = self.state.auto_compact.disabled,
                    error = %err,
                    "proactive auto-compaction failed; continuing turn"
                );
                Ok(())
            }
            Err(err) => {
                self.state.auto_compact.on_real_failure();
                Err(AgentLoopError::ApiError(format!(
                    "auto-compaction failed: {err}"
                )))
            }
        }
    }
}

pub fn compact_json_messages(
    messages: &[serde_json::Value],
    action: CompactAction,
    force: bool,
) -> Result<CompactionOutcome, CompactionError> {
    let compacted = compact_json_messages_apply_with_summary(messages, action, "")?;
    let before = estimate_messages_tokens(messages);
    let after = estimate_messages_tokens(&compacted);
    if compacted.len() == messages.len() && !force {
        return Ok(CompactionOutcome::Skipped {
            reason: SkipReason::NoSafeBoundary,
        });
    }
    Ok(CompactionOutcome::Compacted {
        before_tokens: before,
        after_estimated_tokens: after,
        messages_before: messages.len(),
        messages_after: compacted.len(),
    })
}

pub async fn compact_json_messages_with_provider(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    action: CompactAction,
    force: bool,
) -> Result<(CompactionOutcome, Vec<serde_json::Value>), CompactionError> {
    let summary = generate_compaction_summary_structured(provider, model, messages).await?;
    let compacted = compact_json_messages_apply_with_summary(messages, action, &summary)?;
    let before = estimate_messages_tokens(messages);
    let after = estimate_messages_tokens(&compacted);
    if compacted.len() == messages.len() && !force {
        return Ok((
            CompactionOutcome::Skipped {
                reason: SkipReason::NoSafeBoundary,
            },
            messages.to_vec(),
        ));
    }
    Ok((
        CompactionOutcome::Compacted {
            before_tokens: before,
            after_estimated_tokens: after,
            messages_before: messages.len(),
            messages_after: compacted.len(),
        },
        compacted,
    ))
}

pub async fn generate_compaction_summary_structured(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
) -> Result<String, CompactionError> {
    use crate::commands::build_compact_summary_request;

    let mut context_messages = super::summary_text::to_summary_context_messages(messages);
    for attempt in 0..3 {
        let summary_messages = build_compact_summary_request(&context_messages);
        let request_messages: Vec<serde_json::Value> = summary_messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();
        let request = archon_llm::provider::LlmRequest {
            model: model.to_string(),
            max_tokens: 2048,
            system: vec![serde_json::json!({
                "type": "text",
                "text": archon_context::compact::SUMMARY_PROMPT,
            })],
            messages: request_messages,
            tools: Vec::new(),
            thinking: None,
            speed: Some("fast".to_string()),
            effort: Some("low".to_string()),
            extra: serde_json::Value::Null,
            request_origin: Some("compaction_summary".into()),
            reasoning_encrypted: None,
        };

        let mut rx = match provider.stream(request).await {
            Ok(rx) => rx,
            Err(archon_llm::provider::LlmError::Aborted) => return Err(CompactionError::Cancelled),
            Err(err)
                if err.is_context_window_exceeded()
                    && super::summary_text::trim_oldest_safe_api_round(
                        &mut context_messages,
                        attempt,
                    ) =>
            {
                continue;
            }
            Err(err) => return Err(CompactionError::Provider(err)),
        };
        let mut response = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                archon_llm::streaming::StreamEvent::TextDelta { text, .. } => {
                    response.push_str(&text);
                }
                archon_llm::streaming::StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    if is_cancelled_stream_error(&error_type, &message) {
                        return Err(CompactionError::Cancelled);
                    }
                    let err = classify_stream_error(provider.name(), &error_type, &message);
                    if err.is_context_window_exceeded()
                        && super::summary_text::trim_oldest_safe_api_round(
                            &mut context_messages,
                            attempt,
                        )
                    {
                        response.clear();
                        break;
                    }
                    return Err(CompactionError::Provider(err));
                }
                _ => {}
            }
        }
        let summary = response.trim();
        if !summary.is_empty() {
            return Ok(summary.to_string());
        }
    }
    Err(CompactionError::InvalidSummary(
        "provider returned empty summary".into(),
    ))
}

fn is_cancelled_stream_error(error_type: &str, message: &str) -> bool {
    let error_type = error_type.trim().to_ascii_lowercase();
    if matches!(
        error_type.as_str(),
        "cancelled"
            | "canceled"
            | "user_cancelled"
            | "user_canceled"
            | "client_cancelled"
            | "client_canceled"
            | "operation_cancelled"
            | "operation_canceled"
            | "request_cancelled"
            | "request_canceled"
    ) {
        return true;
    }
    let message = message.trim().to_ascii_lowercase();
    message.contains("cancelled by user")
        || message.contains("canceled by user")
        || message.contains("user cancelled")
        || message.contains("user canceled")
        || message.contains("aborted by user")
        || message.contains("user aborted")
}

pub fn compact_json_messages_apply_with_summary(
    messages: &[serde_json::Value],
    action: CompactAction,
    summary: &str,
) -> Result<Vec<serde_json::Value>, CompactionError> {
    let context_messages = to_context_messages(messages);
    if context_messages.len() < 5 {
        return Err(CompactionError::NoSafeBoundary);
    }
    let summary = if summary.trim().is_empty() {
        "Context Summary: older conversation messages were compacted."
    } else {
        summary
    };
    let compacted = match action {
        CompactAction::Micro => {
            let (msgs, _) = archon_context::microcompact::microcompact_messages(
                &context_messages,
                summary,
                archon_context::compact::DEFAULT_PRESERVE_RECENT_TURNS,
            );
            msgs
        }
        CompactAction::Full => {
            archon_context::compact::compact_messages_default(&context_messages, summary)
        }
    };
    Ok(from_context_messages(&compacted))
}

fn to_context_messages(
    messages: &[serde_json::Value],
) -> Vec<archon_context::messages::ContextMessage> {
    messages
        .iter()
        .map(|m| archon_context::messages::ContextMessage {
            role: m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            content: m.get("content").cloned().unwrap_or(serde_json::Value::Null),
            estimated_tokens: estimate_message_tokens(m),
        })
        .collect()
}

fn from_context_messages(
    messages: &[archon_context::messages::ContextMessage],
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            serde_json::json!({ "role": role, "content": m.content })
        })
        .collect()
}

#[cfg(test)]
#[path = "autocompact_tests.rs"]
mod tests;
