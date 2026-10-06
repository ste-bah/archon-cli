use super::*;

pub(super) async fn collect_stream(rx: Receiver<StreamEvent>) -> Result<LlmResponse> {
    collect_stream_into(rx, None, None).await
}

/// Drain a provider stream into a complete [`LlmResponse`], optionally handing
/// each text delta to `on_text` on the way past.
///
/// One loop understands `StreamEvent` for the whole pipeline. Streaming callers
/// get their tokens from the callback rather than from a parallel consumer,
/// because a `Receiver` has a single consumer and a second one would have to
/// re-implement error classification and usage accounting to match.
pub(super) async fn collect_stream_into(
    mut rx: Receiver<StreamEvent>,
    mut on_text: Option<TextDeltaSink<'_>>,
    progress: Option<archon_shell::progress::Progress>,
) -> Result<LlmResponse> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_uses: Vec<ToolUseEntry> = Vec::new();
    let mut usage = archon_llm::usage::UsageAccumulator::default();
    let mut stop_reason = None;

    // Track in-progress tool_use blocks by content-block index.
    let mut active_tool_blocks: std::collections::HashMap<u32, (String, String, String)> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        if let Some(progress) = &progress {
            progress.record();
        }
        usage.record_event(&event);
        match event {
            StreamEvent::MessageStart { .. } => {}
            StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            } => {
                if block_type == archon_llm::types::ContentBlockType::ToolUse {
                    active_tool_blocks.insert(
                        index,
                        (
                            tool_use_id.unwrap_or_default(),
                            tool_name.unwrap_or_default(),
                            String::new(),
                        ),
                    );
                }
            }
            StreamEvent::TextDelta { text, .. } => {
                if let Some(on_text) = on_text.as_deref_mut() {
                    on_text(&text)?;
                }
                text_parts.push(text);
            }
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => {
                if let Some(entry) = active_tool_blocks.get_mut(&index) {
                    entry.2.push_str(&partial_json);
                }
            }
            StreamEvent::ContentBlockStop { index } => {
                if let Some((_id, name, json_str)) = active_tool_blocks.remove(&index) {
                    let input: serde_json::Value =
                        serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
                    tool_uses.push(ToolUseEntry {
                        tool_name: name,
                        input,
                        output: serde_json::Value::Null,
                    });
                }
            }
            StreamEvent::MessageDelta {
                stop_reason: event_stop_reason,
                ..
            } => {
                if event_stop_reason.is_some() {
                    stop_reason = event_stop_reason;
                }
            }
            StreamEvent::ThinkingDelta { .. }
            | StreamEvent::SignatureDelta { .. }
            | StreamEvent::ReasoningEncrypted { .. }
            | StreamEvent::MessageStop
            | StreamEvent::Ping => {}
            StreamEvent::Error {
                error_type,
                message,
            } => {
                if error_type == "transport_idle" {
                    return Err(anyhow::Error::new(
                        archon_llm::transport_idle::TransportIdle,
                    ));
                }
                let partial_hash = if text_parts.is_empty() {
                    "none".to_string()
                } else {
                    let partial = text_parts.join("");
                    let digest = Sha256::digest(partial.as_bytes());
                    hex::encode(digest)
                };
                if let Some(err) = archon_llm::context_window::classify_context_window_error(
                    None,
                    Some(&error_type),
                    None,
                    &message,
                    Some("pipeline"),
                    None,
                ) {
                    return Err(anyhow::Error::new(err));
                }
                anyhow::bail!(
                    "LLM stream error ({error_type}): {message}; partial_output_hash={partial_hash}"
                );
            }
        }
    }

    Ok(LlmResponse {
        content: text_parts.join(""),
        tool_uses,
        tokens_in: usage.context_input_tokens,
        tokens_out: usage.output_tokens,
        stop_reason,
    })
}
