use std::collections::BTreeMap;

use super::*;

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
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    reactive_overflow_retried: &mut bool,
    reactive_rate_limit_retried: &mut bool,
    last_known_context_tokens: &mut u64,
    request: LlmRequest,
    request_body_bytes: usize,
    large_retry_body_bytes: usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) -> anyhow::Result<StreamRoundResult> {
    let mut rx = open_stream_with_retries(
        runner,
        messages,
        auto_compact,
        reactive_overflow_retried,
        reactive_rate_limit_retried,
        last_known_context_tokens,
        request,
        request_body_bytes,
        large_retry_body_bytes,
        telemetry,
    )
    .await?;

    let mut text_content = String::new();
    let mut thinking_blocks = BTreeMap::<u32, PendingThinkingBlock>::new();
    let mut reasoning_encrypted: Option<String> = None;
    let mut pending_tools: Vec<PendingTool> = Vec::new();
    let mut pending_tool_indices: Vec<u32> = Vec::new();
    let mut usage_acc = archon_llm::usage::UsageAccumulator::default();
    let mut retry_after_compact = false;

    while let Some(event) = rx.recv().await {
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
            } => {
                if handle_stream_error(
                    runner,
                    messages,
                    auto_compact,
                    reactive_overflow_retried,
                    reactive_rate_limit_retried,
                    last_known_context_tokens,
                    request_body_bytes,
                    large_retry_body_bytes,
                    telemetry,
                    error_type,
                    message,
                )
                .await?
                {
                    retry_after_compact = true;
                    break;
                }
            }
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

#[allow(clippy::too_many_arguments)]
async fn open_stream_with_retries(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    reactive_overflow_retried: &mut bool,
    reactive_rate_limit_retried: &mut bool,
    last_known_context_tokens: &mut u64,
    request: LlmRequest,
    request_body_bytes: usize,
    large_retry_body_bytes: usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) -> anyhow::Result<tokio::sync::mpsc::Receiver<StreamEvent>> {
    match runner.provider.stream(request.clone()).await {
        Ok(rx) => Ok(rx),
        Err(e) if e.is_context_window_exceeded() && !*reactive_overflow_retried => {
            *reactive_overflow_retried = true;
            compact_messages_for_retry(
                runner,
                messages,
                auto_compact,
                last_known_context_tokens,
                "reactive subagent compaction failed",
            )
            .await?;
            runner
                .provider
                .stream(LlmRequest {
                    messages: messages.clone(),
                    ..request
                })
                .await
                .map_err(anyhow::Error::new)
        }
        Err(e)
            if crate::agent::autocompact::is_rate_limited_error(&e)
                && !*reactive_rate_limit_retried
                && request_body_bytes >= large_retry_body_bytes =>
        {
            *reactive_rate_limit_retried = true;
            tracing::warn!(
                compaction.reason = "rate_limit_large_request",
                trigger_body_bytes = request_body_bytes,
                threshold_body_bytes = large_retry_body_bytes,
                provider_family = telemetry.provider_family,
                wire_shape = telemetry.wire_shape,
                native_context_window = telemetry.native_context_window,
                runtime_context_budget = telemetry.runtime_context_budget,
                context_source = telemetry.context_source,
                compaction_backend = telemetry.compaction_backend,
                scope = "subagent",
                force = true,
                "rate-limited subagent request is large; compacting before one retry"
            );
            compact_messages_for_retry(
                runner,
                messages,
                auto_compact,
                last_known_context_tokens,
                "rate-limit subagent compaction failed",
            )
            .await?;
            runner
                .provider
                .stream(LlmRequest {
                    messages: messages.clone(),
                    ..request
                })
                .await
                .map_err(anyhow::Error::new)
        }
        Err(e) => Err(anyhow::Error::new(e)),
    }
}

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

#[allow(clippy::too_many_arguments)]
async fn handle_stream_error(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    reactive_overflow_retried: &mut bool,
    reactive_rate_limit_retried: &mut bool,
    last_known_context_tokens: &mut u64,
    request_body_bytes: usize,
    large_retry_body_bytes: usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
    error_type: String,
    message: String,
) -> anyhow::Result<bool> {
    let err = crate::agent::autocompact::classify_stream_error(
        runner.provider.name(),
        &error_type,
        &message,
    );
    if err.is_context_window_exceeded() && !*reactive_overflow_retried {
        *reactive_overflow_retried = true;
        compact_messages_for_retry(
            runner,
            messages,
            auto_compact,
            last_known_context_tokens,
            "reactive subagent compaction failed",
        )
        .await?;
        return Ok(true);
    }
    if crate::agent::autocompact::is_rate_limited_error(&err)
        && !*reactive_rate_limit_retried
        && request_body_bytes >= large_retry_body_bytes
    {
        *reactive_rate_limit_retried = true;
        tracing::warn!(
            compaction.reason = "rate_limit_large_request_stream",
            trigger_body_bytes = request_body_bytes,
            threshold_body_bytes = large_retry_body_bytes,
            provider_family = telemetry.provider_family,
            wire_shape = telemetry.wire_shape,
            native_context_window = telemetry.native_context_window,
            runtime_context_budget = telemetry.runtime_context_budget,
            context_source = telemetry.context_source,
            compaction_backend = telemetry.compaction_backend,
            scope = "subagent",
            force = true,
            "rate-limited subagent stream is large; compacting before one retry"
        );
        compact_messages_for_retry(
            runner,
            messages,
            auto_compact,
            last_known_context_tokens,
            "rate-limit subagent compaction failed",
        )
        .await?;
        return Ok(true);
    }
    runner.emit_activity_stream("error", message, None, true);
    Err(anyhow::Error::new(err))
}

async fn compact_messages_for_retry(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    error_context: &str,
) -> anyhow::Result<()> {
    let (outcome, compacted) = crate::agent::autocompact::compact_json_messages_with_provider(
        runner.provider.as_ref(),
        &runner.model,
        messages,
        crate::agent::CompactAction::Full,
        true,
    )
    .await
    .map_err(|err| anyhow::anyhow!("{error_context}: {err}"))?;
    *messages = compacted;
    let after_current_tokens = match outcome {
        crate::agent::autocompact::CompactionOutcome::Compacted {
            after_estimated_tokens,
            ..
        } => after_estimated_tokens,
        crate::agent::autocompact::CompactionOutcome::Skipped { .. } => {
            crate::agent::autocompact::estimate_messages_tokens(messages)
        }
    };
    *last_known_context_tokens = 0;
    auto_compact.on_success(after_current_tokens);
    Ok(())
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
