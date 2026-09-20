//! Compaction summary generation: the two-pass orchestration, the single-pass
//! sample, and the request bounding they share.
//!
//! Split out of `autocompact.rs` when the two-pass path took that file past the
//! 500-line ceiling. One unit with the compaction types it serves; kept whole
//! rather than divided further so the fallback path from two passes to one
//! stays readable in a single screen.
use super::autocompact::{
    COMPACTION_INPUT_BUDGET_BYTES, CompactionError, GeneratedCompactionSummary,
    classify_stream_error, compaction_attempt_attribution, is_cancelled_stream_error,
};

pub(super) async fn generate_summary_with_usage(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
    preserve_recent: bool,
    summary_max_tokens: u32,
) -> Result<GeneratedCompactionSummary, CompactionError> {
    if let Some(split) =
        super::two_pass::split_for_two_pass(messages, super::two_pass::DEFAULT_SPLIT_FRACTION)
    {
        let pass1 = generate_summary_single_pass(
            provider,
            model,
            split.prefix,
            attribution.clone(),
            preserve_recent,
            summary_max_tokens,
        )
        .await;
        match pass1 {
            Err(CompactionError::Cancelled) => return Err(CompactionError::Cancelled),
            Err(error) => {
                tracing::warn!(%error, "compaction.two_pass: pass 1 failed, falling back to one pass");
            }
            Ok(pass1) => {
                let note = super::two_pass::note_for_pass2(&pass1.text);
                let pass2_input = super::two_pass::build_pass2_messages(&split, &note);
                match generate_summary_single_pass(
                    provider,
                    model,
                    &pass2_input,
                    attribution.clone(),
                    preserve_recent,
                    summary_max_tokens,
                )
                .await
                {
                    Err(CompactionError::Cancelled) => return Err(CompactionError::Cancelled),
                    Err(error) => {
                        tracing::warn!(%error, "compaction.two_pass: pass 2 failed, falling back to one pass");
                    }
                    Ok(mut pass2) => {
                        // Both calls were spent, so both are reported.
                        pass2.input_tokens = pass2.input_tokens.saturating_add(pass1.input_tokens);
                        pass2.output_tokens =
                            pass2.output_tokens.saturating_add(pass1.output_tokens);
                        return Ok(pass2);
                    }
                }
            }
        }
    }
    generate_summary_single_pass(
        provider,
        model,
        messages,
        attribution,
        preserve_recent,
        summary_max_tokens,
    )
    .await
}

async fn generate_summary_single_pass(
    provider: &dyn archon_llm::provider::LlmProvider,
    model: &str,
    messages: &[serde_json::Value],
    attribution: serde_json::Value,
    preserve_recent: bool,
    summary_max_tokens: u32,
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
            max_tokens: summary_max_tokens,
            system: vec![serde_json::json!({
                "type": "text",
                "text": archon_context::compact::SUMMARY_PROMPT,
            })],
            messages: request_messages,
            tools: Default::default(),
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
        let mut stop_reason: Option<String> = None;
        let mut usage = archon_llm::usage::UsageAccumulator::default();
        while let Some(event) = rx.recv().await {
            usage.record_event(&event);
            match event {
                archon_llm::streaming::StreamEvent::TextDelta { text, .. } => {
                    response.push_str(&text);
                }
                archon_llm::streaming::StreamEvent::MessageDelta {
                    stop_reason: Some(ref reason),
                    ..
                } => {
                    stop_reason = Some(reason.clone());
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
        // A summary the model was cut off mid-writing is not a summary. It was
        // being accepted purely because it was non-empty, and `on_success` then
        // reset every failure counter — so the structural-failure suppression
        // below could never trip and compaction retried the same doomed call on
        // every turn. Observed live on 2026-09-08: repeated truncations at the
        // output ceiling, history never shrinking, six context overflows, and a
        // run that never reached implementation.
        //
        // Rejecting is the honest outcome: it counts as a structural failure,
        // and once those exhaust their budget compaction disables itself and
        // `context_fit` drops whole turns instead. Summarise if we can, drop if
        // we cannot, never loop.
        if stop_reason.as_deref() == Some("max_tokens") {
            return Err(CompactionError::InvalidSummary(format!(
                "summary was truncated at the {summary_max_tokens}-token output ceiling \
                 after {} chars; raise [api] max_tokens or reduce the compaction input",
                summary.len()
            )));
        }
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
