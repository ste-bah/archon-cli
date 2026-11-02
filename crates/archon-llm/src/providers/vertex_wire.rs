use super::*;

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse Anthropic Messages API SSE event from Vertex AI (same format as direct Anthropic API).
pub(super) fn parse_anthropic_vertex_event(val: &serde_json::Value) -> Vec<StreamEvent> {
    let event_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let mut events = Vec::new();

    match event_type {
        "message_start" => {
            let message = val.get("message");
            let id = message
                .and_then(|m| m.get("id"))
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let model = message
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let usage = parse_usage(message.and_then(|m| m.get("usage")));
            events.push(StreamEvent::MessageStart { id, model, usage });
        }
        "content_block_start" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let block = val.get("content_block");
            let block_type_str = block
                .and_then(|b| b.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("text");
            let (block_type, tool_use_id, tool_name) = match block_type_str {
                "tool_use" => {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string());
                    (ContentBlockType::ToolUse, id, name)
                }
                _ => (ContentBlockType::Text, None, None),
            };
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            });
        }
        "content_block_delta" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let delta = val.get("delta");
            let delta_type = delta
                .and_then(|d| d.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(StreamEvent::TextDelta { index, text });
                }
                "input_json_delta" => {
                    let partial_json = delta
                        .and_then(|d| d.get("partial_json"))
                        .and_then(|j| j.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(StreamEvent::InputJsonDelta {
                        index,
                        partial_json,
                    });
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            events.push(StreamEvent::ContentBlockStop { index });
        }
        "message_delta" => {
            let delta = val.get("delta");
            let stop_reason = delta
                .and_then(|d| d.get("stop_reason"))
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            let usage = parse_usage(val.get("usage"));
            events.push(StreamEvent::MessageDelta {
                stop_reason,
                usage: Some(usage),
            });
        }
        "message_stop" => {
            events.push(StreamEvent::MessageStop);
        }
        "ping" => {
            events.push(StreamEvent::Ping);
        }
        _ => {}
    }

    events
}

/// Parse a Gemini streaming response object into StreamEvents.
pub(super) fn parse_gemini_vertex_event(val: &serde_json::Value) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    if let Some(candidates) = val.get("candidates").and_then(|c| c.as_array()) {
        for candidate in candidates {
            if let Some(content) = candidate.get("content")
                && let Some(parts) = content.get("parts").and_then(|p| p.as_array())
            {
                for (i, part) in parts.iter().enumerate() {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str())
                        && !text.is_empty()
                    {
                        events.push(StreamEvent::ContentBlockStart {
                            index: i as u32,
                            block_type: ContentBlockType::Text,
                            tool_use_id: None,
                            tool_name: None,
                        });
                        events.push(StreamEvent::TextDelta {
                            index: i as u32,
                            text: text.to_string(),
                        });
                        events.push(StreamEvent::ContentBlockStop { index: i as u32 });
                    }
                }
            }

            let finish_reason = candidate
                .get("finishReason")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if !finish_reason.is_empty() {
                let stop_reason = match finish_reason {
                    "STOP" => "end_turn",
                    "MAX_TOKENS" => "max_tokens",
                    "SAFETY" => "stop_sequence",
                    other => other,
                };
                events.push(StreamEvent::MessageDelta {
                    stop_reason: Some(stop_reason.to_string()),
                    usage: None,
                });
            }
        }
    }

    events
}

/// Parse a usage object from a Vertex AI response into a `Usage` struct.
pub(super) fn parse_usage(usage_val: Option<&serde_json::Value>) -> Usage {
    match usage_val {
        Some(u) => {
            let input = u.get("input_tokens").and_then(|tokens| tokens.as_u64());
            let output = u.get("output_tokens").and_then(|tokens| tokens.as_u64());
            let cache_creation = u
                .get("cache_creation_input_tokens")
                .and_then(|tokens| tokens.as_u64());
            let cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|tokens| tokens.as_u64());
            Usage {
                input_tokens: input.unwrap_or(0),
                output_tokens: output.unwrap_or(0),
                cache_creation_input_tokens: cache_creation.unwrap_or(0),
                cache_read_input_tokens: cache_read.unwrap_or(0),
                input_tokens_available: input.is_some(),
                output_tokens_available: output.is_some(),
                cache_creation_input_tokens_available: cache_creation.is_some(),
                cache_read_input_tokens_available: cache_read.is_some(),
            }
        }
        None => Usage::default(),
    }
}

// ---------------------------------------------------------------------------
// HTTP error mapping
// ---------------------------------------------------------------------------

pub(super) fn map_http_error(status: u16, body: String) -> LlmError {
    if let Some(err) =
        crate::context_window::classify_context_window_body(status, &body, Some("vertex"), None)
    {
        return err;
    }
    let error = crate::provider_fault::from_status(status, &Default::default(), body.clone());
    if matches!(error, LlmError::Rejected { .. }) {
        return error;
    }
    match status {
        401 | 403 => LlmError::Auth(body.into()),
        429 => LlmError::RateLimited {
            retry_after_secs: 60,
            from_provider: false,
            pipeline_retry_after_secs: None,
            message: crate::provider::DiagnosticMessage::redacted_at_source(&body),
        },
        500 | 503 => LlmError::Overloaded {
            status: Some(status),
            retry_after_secs: None,
            message: crate::provider::DiagnosticMessage::redacted_at_source(&body),
        },
        _ => error,
    }
}

#[cfg(test)]
mod diagnostic_redaction_tests {
    fn convert(status: u16, body: &str) -> crate::provider::LlmError {
        super::map_http_error(status, body.into())
    }
    include!("../provider_fault_redaction_cases.rs");
}
