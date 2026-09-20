use std::collections::BTreeMap;

use super::*;

const STREAM_RECONNECT_BACKOFF: Duration = Duration::from_secs(1);
const STREAM_RETRIES: usize = 3;

/// How long the provider may go silent before the round is abandoned.
///
/// Configurable via `[subagent] stream_idle_timeout_secs`. This is a
/// stalled-provider guard, not a thinking budget: the hardcoded 120s was sixty
/// times tighter than the enclosing `host_call_timeout_secs` stage, and killed
/// a live inventory reducer three turns into its work.
fn stream_idle_timeout(runner: &SubagentRunner) -> Duration {
    Duration::from_secs(
        runner
            .agent_config()
            .subagent_stream_idle_timeout_secs
            .max(1),
    )
}

pub(super) struct StreamRoundResult {
    pub text_content: String,
    pub thinking_blocks: BTreeMap<u32, PendingThinkingBlock>,
    pub pending_tools: Vec<PendingTool>,
    pub reasoning_encrypted: Option<String>,
    pub context_input_tokens: u64,
    pub retry_after_compact: bool,
}

pub(super) async fn collect_stream_round(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    (
        recovery_ladder,
        emergency_projection_pending,
        reactive_rate_limit_retried,
        last_known_context_tokens,
    ): (
        &mut crate::agent::autocompact::RecoveryLadder,
        &mut bool,
        &mut bool,
        &mut u64,
    ),
    template: LlmRequest,
    (request_body_bytes, large_retry_body_bytes): (usize, usize),
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) -> anyhow::Result<StreamRoundResult> {
    let mut reconnects = 0;
    // `request` is the body that actually opened the stream. Mid-stream
    // recovery classifies and measures against it rather than against the
    // template, which carries no messages of its own (#171 part 2).
    let (mut rx, request) = loop {
        let attempt_request = if std::mem::take(emergency_projection_pending) {
            emergency_projected_request(runner, messages.as_slice(), &template)
        } else {
            projected_request(runner, messages.as_slice(), &template)
        };
        match tokio::time::timeout(
            stream_idle_timeout(runner),
            open_stream_with_retries(
                runner,
                messages,
                auto_compact,
                recovery_ladder,
                reactive_rate_limit_retried,
                last_known_context_tokens,
                attempt_request,
                request_body_bytes,
                large_retry_body_bytes,
                telemetry,
            ),
        )
        .await
        {
            Ok(result) => break result?,
            Err(_) if reconnects < STREAM_RETRIES => {
                reconnects += 1;
                tokio::time::sleep(STREAM_RECONNECT_BACKOFF).await;
            }
            Err(_) => {
                anyhow::bail!("Subagent LLM stream idle timeout while opening response")
            }
        }
    };

    let mut text_content = String::new();
    let mut thinking_blocks = BTreeMap::<u32, PendingThinkingBlock>::new();
    let mut reasoning_encrypted: Option<String> = None;
    let mut pending_tools: Vec<PendingTool> = Vec::new();
    let mut pending_tool_indices: Vec<u32> = Vec::new();
    let mut usage_acc = archon_llm::usage::UsageAccumulator::default();
    let mut retry_after_compact = false;
    let mut finish_reason = None;
    let mut terminal_marker = false;

    loop {
        let received = tokio::time::timeout(stream_idle_timeout(runner), rx.recv()).await;
        let transport_failure = matches!(&received, Ok(Some(StreamEvent::Error { error_type, message }))
            if matches!(error_type.as_str(), "network" | "http_error") || error_type == "protocol" && message.contains("stream ended before message_stop"));
        let empty_terminal = text_content.trim().is_empty()
            && pending_tools.is_empty()
            && !matches!(finish_reason.as_deref(), Some("max_tokens" | "length"));
        let interrupted = received.is_err()
            || transport_failure
            || matches!(&received, Ok(None))
                && ((!terminal_marker && finish_reason.is_none()) || empty_terminal);
        if interrupted {
            if reconnects >= STREAM_RETRIES {
                let reason = if received.is_err() {
                    "stream idle timeout"
                } else {
                    "incomplete response"
                };
                anyhow::bail!(
                    "subagent stream retry exhausted after {STREAM_RETRIES} retries: {reason}; prior conversation retained"
                );
            }
            reconnects += 1;
            if let Some(scope) = archon_observability::transport::current() {
                scope.record(serde_json::json!({"kind":"stream_round_retry","attempt":reconnects,
                    "reason":if received.is_err(){"idle_timeout"}else{"incomplete_stream"},
                    "discarded_text_bytes":text_content.len(),"discarded_tool_calls":pending_tools.len()}));
            }
            // No tool has executed until the round completes. Discard ALL partial
            // deltas and resend the identical request, not a fresh agent prompt.
            drop(rx);
            text_content.clear();
            thinking_blocks.clear();
            pending_tools.clear();
            pending_tool_indices.clear();
            reasoning_encrypted = None;
            finish_reason = None;
            terminal_marker = false;
            usage_acc = archon_llm::usage::UsageAccumulator::default();
            loop {
                tokio::time::sleep(STREAM_RECONNECT_BACKOFF * reconnects as u32).await;
                let opened = tokio::time::timeout(
                    stream_idle_timeout(runner),
                    runner.provider.stream(request.clone()),
                )
                .await;
                match opened {
                    Ok(Ok(receiver)) => {
                        rx = receiver;
                        break;
                    }
                    Ok(Err(error)) => return Err(error.into()),
                    Err(_) if reconnects < STREAM_RETRIES => {
                        reconnects += 1;
                    }
                    Err(_) => anyhow::bail!("subagent stream retry exhausted while reconnecting"),
                }
            }
            continue;
        }
        let Some(event) = received.expect("timeout handled above") else {
            break;
        };
        usage_acc.record_event(&event);
        if let StreamEvent::MessageDelta {
            stop_reason: Some(reason),
            ..
        } = &event
        {
            finish_reason = Some(reason.clone());
        }
        if matches!(event, StreamEvent::MessageStop) {
            terminal_marker = true;
        }
        match event {
            StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            } => record_content_block_start(
                runner,
                index,
                block_type,
                tool_use_id,
                tool_name,
                &mut thinking_blocks,
                &mut pending_tools,
                &mut pending_tool_indices,
            ),
            StreamEvent::TextDelta { text, .. } => {
                runner.emit_activity_stream("text", text.clone(), None, false);
                text_content.push_str(&text);
            }
            StreamEvent::ThinkingDelta { index, thinking } => {
                thinking_blocks
                    .entry(index)
                    .or_default()
                    .thinking
                    .push_str(&thinking);
                runner.emit_activity_stream("thinking", thinking, None, false);
            }
            StreamEvent::SignatureDelta { index, signature } => {
                thinking_blocks
                    .entry(index)
                    .or_default()
                    .signature
                    .push_str(&signature);
            }
            StreamEvent::ReasoningEncrypted { blob } => {
                reasoning_encrypted = Some(blob);
            }
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => append_tool_input_delta(
                index,
                &partial_json,
                &mut pending_tools,
                &pending_tool_indices,
            ),
            StreamEvent::ContentBlockStop { .. } => {}
            StreamEvent::Error {
                error_type,
                message,
            } if handle_stream_error(
                runner,
                messages,
                auto_compact,
                recovery_ladder,
                emergency_projection_pending,
                reactive_rate_limit_retried,
                last_known_context_tokens,
                request_body_bytes,
                large_retry_body_bytes,
                telemetry,
                &request,
                error_type.clone(),
                message.clone(),
            )
            .await? =>
            {
                retry_after_compact = true;
                break;
            }
            StreamEvent::Error { .. } => {}
            StreamEvent::MessageStart { ref usage, .. } => {
                record_message_start_usage(runner, usage);
            }
            StreamEvent::MessageDelta {
                usage: Some(ref usage),
                ..
            } => {
                record_message_delta_usage(runner, usage);
            }
            _ => {}
        }
    }

    if !retry_after_compact && text_content.trim().is_empty() && pending_tools.is_empty() {
        let reason = finish_reason.as_deref().unwrap_or("unavailable");
        if let Some(scope) = archon_observability::transport::current() {
            scope.record(
                serde_json::json!({"kind":"empty_reply", "finish_reason":reason,
                "terminal_marker":terminal_marker,"thinking_blocks":thinking_blocks.len(),
                "text_bytes":text_content.len(),"tool_calls":pending_tools.len()}),
            );
        }
        anyhow::bail!(
            "the provider returned an empty reply; finish_reason={reason}; terminal_marker={terminal_marker}; thinking_blocks={}",
            thinking_blocks.len()
        );
    }
    Ok(StreamRoundResult {
        text_content,
        thinking_blocks,
        pending_tools,
        reasoning_encrypted,
        context_input_tokens: usage_acc.context_input_tokens,
        retry_after_compact,
    })
}

#[path = "stream_round_recovery.rs"]
mod recovery;
#[cfg(test)]
use recovery::compact_messages_for_retry;
use recovery::{
    emergency_projected_request, handle_stream_error, open_stream_with_retries, projected_request,
};

#[allow(clippy::too_many_arguments)]
fn record_content_block_start(
    runner: &SubagentRunner,
    index: u32,
    block_type: ContentBlockType,
    tool_use_id: Option<String>,
    tool_name: Option<String>,
    thinking_blocks: &mut BTreeMap<u32, PendingThinkingBlock>,
    pending_tools: &mut Vec<PendingTool>,
    pending_tool_indices: &mut Vec<u32>,
) {
    if block_type == ContentBlockType::ToolUse {
        let name = tool_name.unwrap_or_default();
        runner.emit_activity_stream("tool_call", format!("calling {name}"), Some(&name), false);
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

fn append_tool_input_delta(
    index: u32,
    partial_json: &str,
    pending_tools: &mut [PendingTool],
    pending_tool_indices: &[u32],
) {
    if !crate::agent::tool_input_json::append_delta_by_index(
        pending_tools,
        pending_tool_indices,
        index,
        partial_json,
        |tool, delta| tool.input_json.push_str(delta),
    ) {
        tracing::warn!(
            tool_block_index = index,
            scope = "subagent",
            "received tool input JSON delta without matching tool block"
        );
    }
}

fn record_message_start_usage(runner: &SubagentRunner, usage: &archon_llm::types::Usage) {
    if let Some(ref tracker) = runner.progress
        && let Ok(mut guard) = tracker.lock()
    {
        guard.cumulative_input_tokens += usage.input_tokens;
        guard.cumulative_output_tokens += usage.output_tokens;
        guard.cumulative_cache_creation_tokens += usage.cache_creation_input_tokens;
        guard.cumulative_cache_read_tokens += usage.cache_read_input_tokens;
        guard.last_update = chrono::Utc::now();
    }
}

fn record_message_delta_usage(runner: &SubagentRunner, usage: &archon_llm::types::Usage) {
    if let Some(ref tracker) = runner.progress
        && let Ok(mut guard) = tracker.lock()
    {
        guard.cumulative_output_tokens += usage.output_tokens;
        guard.last_update = chrono::Utc::now();
    }
}

#[cfg(test)]
mod tests {
    include!("stream_round_tests.rs");
}
