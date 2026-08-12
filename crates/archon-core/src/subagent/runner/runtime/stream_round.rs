use std::collections::BTreeMap;

use super::*;

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const STREAM_RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

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
    request: LlmRequest,
    (request_body_bytes, large_retry_body_bytes): (usize, usize),
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) -> anyhow::Result<StreamRoundResult> {
    let mut reconnected = false;
    let mut rx = loop {
        let attempt_request = if std::mem::take(emergency_projection_pending) {
            emergency_projected_request(runner, messages.as_slice(), &request)
        } else {
            projected_request(runner, messages.as_slice(), &request)
        };
        match tokio::time::timeout(
            STREAM_IDLE_TIMEOUT,
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
            Err(_) if !reconnected => {
                reconnected = true;
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
    let mut received_event = false;

    loop {
        let event = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, rx.recv()).await {
            Ok(event) => event,
            Err(_) if !received_event && !reconnected => {
                drop(rx);
                reconnected = true;
                tokio::time::sleep(STREAM_RECONNECT_BACKOFF).await;
                let retry_request = projected_request(runner, messages.as_slice(), &request);
                rx = tokio::time::timeout(
                    STREAM_IDLE_TIMEOUT,
                    runner.provider.stream(retry_request),
                )
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Subagent LLM stream idle timeout while reconnecting")
                })??;
                continue;
            }
            Err(_) => {
                anyhow::bail!(
                    "Subagent LLM stream idle timeout: no event received for {}s",
                    STREAM_IDLE_TIMEOUT.as_secs()
                )
            }
        };
        let Some(event) = event else {
            break;
        };
        received_event = true;
        usage_acc.record_event(&event);
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
