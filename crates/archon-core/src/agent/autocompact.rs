use super::*;

#[path = "request_pressure.rs"]
mod request_pressure;
pub(crate) use request_pressure::*;
#[path = "autocompact_agent.rs"]
mod agent_impl;

#[path = "segment_compaction.rs"]
mod segment_compaction;
pub use segment_compaction::*;
#[path = "segment_compaction_validation.rs"]
mod segment_compaction_validation;
pub use segment_compaction_validation::validate_compaction_source;

#[path = "autocompact_recovery.rs"]
mod recovery;
#[cfg(test)]
use recovery::MAX_COMPACT_FAILURES;
pub use recovery::*;

const MICRO_COMPACT_FRACTION: f32 = 0.65;
const COMPACTION_INPUT_BUDGET_BYTES: usize = 320_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactAction {
    Micro,
    Full,
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
    attribution: serde_json::Value,
) -> Result<(CompactionOutcome, Vec<serde_json::Value>), CompactionError> {
    let summary =
        generate_compaction_summary_structured(provider, model, messages, attribution).await?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCompactionSummary {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub async fn generate_compaction_summary_structured(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
) -> Result<String, CompactionError> {
    generate_compaction_summary_with_usage(provider, model, messages, attribution)
        .await
        .map(|summary| summary.text)
}

pub async fn generate_compaction_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
) -> Result<GeneratedCompactionSummary, CompactionError> {
    generate_summary_with_usage(provider, model, messages, attribution, true).await
}

pub async fn generate_segment_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
) -> Result<GeneratedCompactionSummary, CompactionError> {
    generate_summary_with_usage(provider, model, messages, attribution, false).await
}

async fn generate_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
    preserve_recent: bool,
) -> Result<GeneratedCompactionSummary, CompactionError> {
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
        let summary_messages = if preserve_recent {
            build_compact_summary_request(&context_messages)
        } else {
            archon_context::compact::build_summary_request(&context_messages, 0)
        };
        let request_messages = bound_summary_request_messages(summary_messages)?;
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
            extra: compaction_attempt_attribution(&attribution, attempt as u64),
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
        let mut usage = archon_llm::usage::UsageAccumulator::default();
        while let Some(event) = rx.recv().await {
            usage.record_event(&event);
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
            return Ok(GeneratedCompactionSummary {
                text: summary.to_string(),
                input_tokens: usage.context_input_tokens,
                output_tokens: usage.output_tokens,
            });
        }
    }
    Err(CompactionError::InvalidSummary(
        "provider returned empty summary".into(),
    ))
}

fn bound_summary_request_messages(
    messages: Vec<archon_context::messages::ContextMessage>,
) -> Result<Vec<serde_json::Value>, CompactionError> {
    let mut request_messages: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|message| serde_json::json!({ "role": message.role, "content": message.content }))
        .collect();
    if serialized_summary_request_len(&request_messages)? <= COMPACTION_INPUT_BUDGET_BYTES {
        return Ok(request_messages);
    }

    let string_content_count = request_messages
        .iter()
        .filter(|message| {
            message
                .get("content")
                .is_some_and(serde_json::Value::is_string)
        })
        .count()
        .max(1);
    let overhead = serialized_summary_request_overhead(&request_messages)?;
    let content_budget = COMPACTION_INPUT_BUDGET_BYTES.saturating_sub(overhead);
    let per_message_budget = content_budget / string_content_count + 2;
    for message in &mut request_messages {
        let Some(content) = message.get("content").and_then(serde_json::Value::as_str) else {
            continue;
        };
        message["content"] = serde_json::json!(
            super::tool_result_context::cap_tool_output_to_bytes(content, per_message_budget)
                .content
        );
    }

    if serialized_summary_request_len(&request_messages)? > COMPACTION_INPUT_BUDGET_BYTES {
        return Err(CompactionError::InvalidSummary(format!(
            "summary request exceeds {COMPACTION_INPUT_BUDGET_BYTES}-byte input budget"
        )));
    }
    Ok(request_messages)
}

fn serialized_summary_request_len(
    messages: &[serde_json::Value],
) -> Result<usize, CompactionError> {
    serde_json::to_vec(messages)
        .map(|messages| messages.len())
        .map_err(|error| CompactionError::InvalidSummary(error.to_string()))
}

fn serialized_summary_request_overhead(
    messages: &[serde_json::Value],
) -> Result<usize, CompactionError> {
    let mut messages = messages.to_vec();
    for message in &mut messages {
        if message
            .get("content")
            .is_some_and(serde_json::Value::is_string)
        {
            message["content"] = serde_json::Value::String(String::new());
        }
    }
    serialized_summary_request_len(&messages)
}

fn compaction_attempt_attribution(base: &serde_json::Value, round: u64) -> serde_json::Value {
    let mut attribution = base.clone();
    attribution["archon_runtime"]["round"] = serde_json::json!(round);
    attribution
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
#[path = "autocompact_attribution_tests.rs"]
mod attribution_tests;
#[cfg(test)]
#[path = "autocompact_recovery_tests.rs"]
mod recovery_tests;
#[cfg(test)]
#[path = "segment_compaction_tests.rs"]
mod segment_compaction_tests;
#[cfg(test)]
#[path = "autocompact_tests.rs"]
mod tests;
