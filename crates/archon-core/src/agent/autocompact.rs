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
pub(super) const COMPACTION_INPUT_BUDGET_BYTES: usize = 320_000;

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
    summary_max_tokens: u32,
) -> Result<(CompactionOutcome, Vec<serde_json::Value>), CompactionError> {
    let summary = generate_compaction_summary_structured(
        provider,
        model,
        messages,
        attribution,
        summary_max_tokens,
    )
    .await?;
    let compacted = compact_json_messages_apply_with_summary(messages, action, &summary)?;
    let before = estimate_messages_tokens(messages);
    let after = estimate_messages_tokens(&compacted);
    // A summary can be well-formed, complete and still achieve nothing — when
    // the history is mostly turns compaction preserves, the replacement is no
    // smaller than what it replaced. That was being reported as success, so
    // `on_success` reset every failure counter and the next turn compacted
    // again, indefinitely, each attempt costing a model call and reclaiming
    // nothing.
    //
    // Checked on the outcome rather than on the summary text: measuring whether
    // the conversation actually shrank catches cases no inspection of the
    // summary can, because the summary itself is fine. An error rather than a
    // skip, because `NoSafeBoundary` deliberately clears the failure count and
    // this must accumulate toward suppression instead.
    if after >= before && !force {
        return Err(CompactionError::InvalidSummary(format!(
            "compaction reclaimed nothing: {before} tokens before, {after} after"
        )));
    }
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
    summary_max_tokens: u32,
) -> Result<String, CompactionError> {
    generate_compaction_summary_with_usage(provider, model, messages, attribution, summary_max_tokens)
        .await
        .map(|summary| summary.text)
}

pub async fn generate_compaction_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
    summary_max_tokens: u32,
) -> Result<GeneratedCompactionSummary, CompactionError> {
    super::autocompact_summary::generate_summary_with_usage(provider, model, messages, attribution, true, summary_max_tokens).await
}

pub async fn generate_segment_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
    summary_max_tokens: u32,
) -> Result<GeneratedCompactionSummary, CompactionError> {
    super::autocompact_summary::generate_summary_with_usage(provider, model, messages, attribution, false, summary_max_tokens).await
}

/// Summarise, hierarchically when the conversation is long enough to benefit.
///
/// Pass 1 compresses the older ~95% by token weight; pass 2 merges that note
/// with the recent tail. One pass over everything spreads the budget evenly and
/// loses the recent work first, which is the part a successor needs most.
///
/// Any pass-1 or pass-2 failure falls back to a single pass over the whole
/// conversation rather than failing the compaction: two-pass is a quality
/// improvement, and trading a worse summary for no summary would be a bad deal.
/// Cancellation is not a failure and propagates immediately.
pub(super) fn compaction_attempt_attribution(base: &serde_json::Value, round: u64) -> serde_json::Value {
    let mut attribution = base.clone();
    attribution["archon_runtime"]["round"] = serde_json::json!(round);
    attribution
}

pub(super) fn is_cancelled_stream_error(error_type: &str, message: &str) -> bool {
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
