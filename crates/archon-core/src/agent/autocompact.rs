use super::*;

#[path = "request_pressure.rs"]
mod request_pressure;
pub(crate) use request_pressure::*;
#[path = "autocompact_agent.rs"]
mod agent_impl;

const MICRO_COMPACT_FRACTION: f32 = 0.65;
const MAX_COMPACT_FAILURES: u32 = 3;
const COMPACTION_INPUT_BUDGET_BYTES: usize = 320_000;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactionTelemetry {
    pub provider_family: &'static str,
    pub wire_shape: &'static str,
    pub native_context_window: u64,
    pub runtime_context_budget: u64,
    pub context_source: &'static str,
    pub compaction_backend: &'static str,
}

pub(crate) fn compaction_telemetry(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    override_window: Option<u64>,
    work_dir: &std::path::Path,
) -> CompactionTelemetry {
    let resolution = archon_llm::context_window::resolve_context_window_for_work_dir(
        model,
        override_window,
        Some(provider),
        Some(work_dir),
    );
    let policy = provider.compaction_policy();
    CompactionTelemetry {
        provider_family: policy.provider_family.label(),
        wire_shape: policy.wire_shape.label(),
        native_context_window: resolution.context_window,
        runtime_context_budget: resolution
            .runtime_context_budget
            .unwrap_or(resolution.context_window),
        context_source: resolution.source.label(),
        compaction_backend: policy.backend.label(),
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

    let mut working_messages = messages.to_vec();
    let dropped = super::summary_text::trim_raw_to_compaction_budget(
        &mut working_messages,
        COMPACTION_INPUT_BUDGET_BYTES,
    );
    if dropped > 0 {
        tracing::info!(
            dropped_messages = dropped,
            remaining = working_messages.len(),
            budget_bytes = COMPACTION_INPUT_BUDGET_BYTES,
            "compaction.pre_trim: bounded summary input"
        );
    }

    let mut context_messages = super::summary_text::to_summary_context_messages(&working_messages);
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
            speed: None,
            effort: None,
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
