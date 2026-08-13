//! Wire-format helpers for the Bedrock Converse API.
//!
//! Conversion from Archon message/content shapes into Bedrock Converse bodies,
//! extraction and parsing of streamed Converse events, and HTTP error mapping.
//! Split out of `bedrock.rs` to keep both modules readable.

use crate::provider::LlmError;
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

// ---------------------------------------------------------------------------
// Message conversion: Archon → Bedrock Converse format
// ---------------------------------------------------------------------------

pub(super) fn convert_message_to_bedrock(msg: &serde_json::Value) -> serde_json::Value {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

    // Map content blocks.
    if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
        let bedrock_content: Vec<serde_json::Value> = content_arr
            .iter()
            .filter_map(convert_content_block)
            .collect();

        serde_json::json!({
            "role": role,
            "content": bedrock_content
        })
    } else if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
        serde_json::json!({
            "role": role,
            "content": [{"text": content_str}]
        })
    } else {
        serde_json::json!({
            "role": role,
            "content": []
        })
    }
}

fn convert_content_block(block: &serde_json::Value) -> Option<serde_json::Value> {
    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match block_type {
        "text" => {
            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
            Some(serde_json::json!({"text": text}))
        }
        "tool_use" => {
            let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let input = block
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Some(serde_json::json!({
                "toolUse": {
                    "toolUseId": id,
                    "name": name,
                    "input": input
                }
            }))
        }
        "tool_result" => {
            let tool_use_id = block
                .get("tool_use_id")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let content = block.get("content").and_then(|c| c.as_str()).unwrap_or("");
            Some(serde_json::json!({
                "toolResult": {
                    "toolUseId": tool_use_id,
                    "content": [{"text": content}]
                }
            }))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Bedrock event parsing
// ---------------------------------------------------------------------------

/// Extract complete JSON objects from a buffer of text.
/// Returns (events, bytes_consumed).
pub(super) fn extract_bedrock_events(text: &str) -> (Vec<serde_json::Value>, usize) {
    let mut events = Vec::new();
    let mut consumed = 0;
    let bytes = text.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace and newlines.
        while pos < bytes.len()
            && (bytes[pos] == b'\r' || bytes[pos] == b'\n' || bytes[pos] == b' ')
        {
            pos += 1;
        }

        if pos >= bytes.len() {
            break;
        }

        // Find a JSON object (starting with '{').
        if bytes[pos] != b'{' {
            break;
        }

        // Try to find the end of this JSON object.
        if let Some(end) = find_json_object_end(bytes, pos) {
            let slice = &text[pos..=end];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(slice) {
                events.push(val);
                consumed = end + 1;
            }
            pos = end + 1;
        } else {
            // Incomplete JSON — stop here.
            break;
        }
    }

    (events, consumed)
}

/// Find the end index of a JSON object starting at `start`.
fn find_json_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    let mut i = start;

    while i < bytes.len() {
        let b = bytes[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            match b {
                b'\\' => escape_next = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }

        i += 1;
    }

    None
}

/// Parse a Bedrock Converse stream event JSON value into StreamEvent(s).
pub fn parse_bedrock_event(event: &serde_json::Value) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    if let Some(start) = event.get("contentBlockStart") {
        let index = start
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        let block = start.get("contentBlock");
        let has_tool_use = block.and_then(|b| b.get("toolUse")).is_some();

        if has_tool_use {
            let tool_use = block.and_then(|b| b.get("toolUse"));
            let tool_id = tool_use
                .and_then(|t| t.get("toolUseId"))
                .and_then(|i| i.as_str())
                .map(|s| s.to_string());
            let tool_name = tool_use
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: tool_id,
                tool_name,
            });
        } else {
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            });
        }
    }

    if let Some(delta_obj) = event.get("contentBlockDelta") {
        let index = delta_obj
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        let delta = delta_obj.get("delta");

        if let Some(text) = delta.and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            events.push(StreamEvent::TextDelta {
                index,
                text: text.to_string(),
            });
        } else if let Some(json_str) = delta
            .and_then(|d| d.get("toolUse"))
            .and_then(|t| t.get("input"))
            .and_then(|i| i.as_str())
        {
            events.push(StreamEvent::InputJsonDelta {
                index,
                partial_json: json_str.to_string(),
            });
        }
    }

    if let Some(stop_obj) = event.get("contentBlockStop") {
        let index = stop_obj
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        events.push(StreamEvent::ContentBlockStop { index });
    }

    if let Some(msg_delta) = event.get("messageStop") {
        let stop_reason = msg_delta
            .get("stopReason")
            .and_then(|r| r.as_str())
            .map(|s| {
                // Normalize Bedrock stop reasons to Anthropic conventions.
                match s {
                    "end_turn" => "end_turn",
                    "tool_use" => "tool_use",
                    "max_tokens" => "max_tokens",
                    other => other,
                }
                .to_string()
            });
        events.push(StreamEvent::MessageDelta {
            stop_reason,
            usage: None,
        });
        events.push(StreamEvent::MessageStop);
    }

    if let Some(metadata) = event.get("metadata")
        && let Some(usage) = metadata.get("usage")
    {
        let input_tokens = usage.get("inputTokens").and_then(|t| t.as_u64());
        let output_tokens = usage.get("outputTokens").and_then(|t| t.as_u64());
        // Bedrock names these differently from Anthropic — camelCase, and
        // `creation` is `write`. They were previously hardcoded to zero, which
        // made a cache that was never working indistinguishable from one that
        // was working perfectly.
        let cache_write = usage.get("cacheWriteInputTokens").and_then(|t| t.as_u64());
        let cache_read = usage.get("cacheReadInputTokens").and_then(|t| t.as_u64());
        events.push(StreamEvent::MessageDelta {
            stop_reason: None,
            usage: Some(Usage {
                // `inputTokens` on Bedrock counts only the tokens that were
                // NOT served from or written to cache, unlike Anthropic where
                // it is the total. Reporting it verbatim would under-count the
                // real prompt size by exactly the amount caching is saving —
                // so the moment caching starts working, usage would appear to
                // collapse rather than shift between categories.
                input_tokens: input_tokens.unwrap_or(0)
                    + cache_read.unwrap_or(0)
                    + cache_write.unwrap_or(0),
                output_tokens: output_tokens.unwrap_or(0),
                cache_creation_input_tokens: cache_write.unwrap_or(0),
                cache_read_input_tokens: cache_read.unwrap_or(0),
                input_tokens_available: input_tokens.is_some(),
                output_tokens_available: output_tokens.is_some(),
                cache_creation_input_tokens_available: cache_write.is_some(),
                cache_read_input_tokens_available: cache_read.is_some(),
            }),
        });
    }

    events
}

// ---------------------------------------------------------------------------
// HTTP error mapping
// ---------------------------------------------------------------------------

pub(super) fn map_http_error(status: u16, body: String) -> LlmError {
    if let Some(err) =
        crate::context_window::classify_context_window_body(status, &body, Some("bedrock"), None)
    {
        return err;
    }
    match status {
        400 => LlmError::Server {
            status,
            message: format!("Bad request: {body}"),
        },
        401 | 403 => LlmError::Auth(body),
        429 => LlmError::RateLimited {
            retry_after_secs: 60,
        },
        500 | 503 => LlmError::Overloaded,
        _ => LlmError::Server {
            status,
            message: body,
        },
    }
}
