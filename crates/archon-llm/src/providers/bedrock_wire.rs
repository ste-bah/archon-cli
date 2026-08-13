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

/// Convert one Archon message to Converse shape, or `None` if it has no content
/// Bedrock will accept.
///
/// Returning `Option` rather than a message with an empty `content` array is the
/// whole point. Converse rejects an empty content field outright:
///
/// ```text
/// The content field in the Message object at messages.1 is empty.
/// Add a ContentBlock object to the content field and try again.
/// ```
///
/// and it rejects the *request*, not the message — so a single empty assistant
/// turn poisons the conversation permanently. Every later turn replays it and
/// fails, which is what took a working session down after one bad round.
///
/// `convert_content_block` drops any block type it does not recognise, so a
/// message whose blocks are all unrecognised silently became that empty array.
/// An assistant turn carrying only thinking was exactly that case.
pub(super) fn convert_message_to_bedrock(msg: &serde_json::Value) -> Option<serde_json::Value> {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

    // Map content blocks.
    let content = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
        content_arr
            .iter()
            .filter_map(convert_content_block)
            .collect::<Vec<_>>()
    } else if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
        // A plain-string content field is Archon's other shape; an empty string
        // is still no content as far as Converse is concerned.
        if content_str.is_empty() {
            Vec::new()
        } else {
            vec![serde_json::json!({"text": content_str})]
        }
    } else {
        Vec::new()
    };

    if content.is_empty() {
        tracing::debug!(
            role,
            "dropping message with no Bedrock-representable content; sending it \
             would fail the whole request"
        );
        return None;
    }

    Some(serde_json::json!({
        "role": role,
        "content": content
    }))
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
        // Extended thinking. Archon keeps these on assistant turns whenever a
        // thinking budget is configured, and dropping them was what produced
        // empty assistant messages: a turn that thought and then called a tool
        // has no `text` block at all, so every block was discarded.
        //
        // Converse carries them as `reasoningContent`. Round-tripping them also
        // preserves the signature, which the model needs to verify its own
        // earlier reasoning.
        "thinking" => {
            let text = block
                .get("thinking")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            let mut reasoning = serde_json::json!({"text": text});
            if let Some(sig) = block.get("signature").and_then(|s| s.as_str()) {
                reasoning["signature"] = serde_json::json!(sig);
            }
            Some(serde_json::json!({
                "reasoningContent": {"reasoningText": reasoning}
            }))
        }
        "redacted_thinking" => block
            .get("data")
            .and_then(|d| d.as_str())
            .map(|data| serde_json::json!({"reasoningContent": {"redactedContent": data}})),
        other => {
            // Unrecognised blocks are still dropped — sending a shape Converse
            // does not know is a 400 on every turn — but no longer in silence.
            tracing::debug!(block_type = other, "dropping unsupported content block");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Bedrock event parsing
// ---------------------------------------------------------------------------

/// Decode `application/vnd.amazon.eventstream` frames into wrapped events.
///
/// This is what `ConverseStream` actually returns, and why the streaming path
/// produced nothing: the response is **binary framed**, not a JSON stream. Each
/// message is
///
/// ```text
/// [ total_len u32 | headers_len u32 | prelude_crc u32 ]   <- 12-byte prelude
/// [ headers ... headers_len bytes                    ]
/// [ payload ... total_len - headers_len - 16 bytes   ]
/// [ message_crc u32                                  ]
/// ```
///
/// and the event name lives in the `:event-type` **header**, not in the payload.
/// The payload for a text delta is bare:
///
/// ```json
/// {"contentBlockIndex":0,"delta":{"text":"hi"}}
/// ```
///
/// while [`parse_bedrock_event`] looks for `event.get("contentBlockDelta")`. So
/// scanning the bytes for JSON found the payloads but never a wrapper key, every
/// lookup missed, and every turn completed with no content — in silence, since
/// nothing errored.
///
/// This re-wraps each payload under its header name, restoring the shape
/// `parse_bedrock_event` already expects.
///
/// Returns `(events, bytes_consumed)`; a partial trailing frame is left in the
/// buffer for the next chunk.
pub(super) fn decode_eventstream_frames(buf: &[u8]) -> (Vec<serde_json::Value>, usize) {
    let mut events = Vec::new();
    let mut pos = 0usize;

    while buf.len() >= pos + 12 {
        let total_len = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
        let headers_len =
            u32::from_be_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]);

        let total_len = total_len as usize;
        let headers_len = headers_len as usize;

        // A frame is prelude(12) + headers + payload + crc(4). Guard against a
        // corrupt length rather than panicking on a slice out of range.
        if total_len < 16 + headers_len || total_len > 16 * 1024 * 1024 {
            // Unrecoverable framing: consume what we have so the caller does not
            // spin on the same bytes forever.
            return (events, buf.len());
        }
        if buf.len() < pos + total_len {
            break; // partial frame; wait for more bytes
        }

        let headers = &buf[pos + 12..pos + 12 + headers_len];
        let payload = &buf[pos + 12 + headers_len..pos + total_len - 4];

        if let Some(event_type) = eventstream_header(headers, ":event-type")
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload)
        {
            events.push(serde_json::json!({ event_type: value }));
        }

        pos += total_len;
    }

    (events, pos)
}

/// Read a string-valued header out of an event-stream header block.
///
/// Header layout: name_len(u8), name, value_type(u8), then for the string type
/// (7) a u16 length followed by the bytes. Other value types are skipped by
/// their fixed widths so a later header can still be found.
fn eventstream_header(mut headers: &[u8], wanted: &str) -> Option<String> {
    while !headers.is_empty() {
        let name_len = *headers.first()? as usize;
        if headers.len() < 1 + name_len + 1 {
            return None;
        }
        let name = std::str::from_utf8(&headers[1..1 + name_len]).ok()?;
        let value_type = headers[1 + name_len];
        let rest = &headers[1 + name_len + 1..];

        let (value, consumed) = match value_type {
            // 7 = string, the only type Bedrock uses for :event-type.
            7 => {
                if rest.len() < 2 {
                    return None;
                }
                let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                if rest.len() < 2 + len {
                    return None;
                }
                (
                    Some(std::str::from_utf8(&rest[2..2 + len]).ok()?.to_string()),
                    2 + len,
                )
            }
            0 | 1 => (None, 0), // bool true/false, no value bytes
            2 => (None, 1),     // byte
            3 => (None, 2),     // short
            4 => (None, 4),     // integer
            5 | 8 => (None, 8), // long, timestamp
            6 => {
                if rest.len() < 2 {
                    return None;
                }
                let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                (None, 2 + len) // byte array
            }
            9 => (None, 16), // uuid
            _ => return None,
        };

        if name == wanted
            && let Some(v) = value
        {
            return Some(v);
        }
        if rest.len() < consumed {
            return None;
        }
        headers = &rest[consumed..];
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
                // NOT served from or written to cache — verified live at
                // `inputTokens: 3` on a 4,424-token request. That is already
                // the DISJOINT form `UsageAccumulator` expects, the same shape
                // the Anthropic Messages API reports, so it is passed through
                // untouched.
                //
                // This used to add `cache_read` and `cache_write` back in, to
                // stop the context figure collapsing the moment caching started
                // working. The intent was right and the mechanism was wrong:
                // the accumulator computes the context as
                // `input + cache_creation + cache_read`, so folding them into
                // `input` as well counted every cached token TWICE. Measured on
                // a live turn: a ~12,091-token prompt reported 28,255 tokens of
                // context, and the cached tokens were charged at the full input
                // rate on top of the cache-read rate.
                //
                // Leaving it disjoint achieves what the old comment wanted
                // anyway — the accumulator's sum is the true total, so the
                // context does not collapse, it moves between buckets.
                input_tokens: input_tokens.unwrap_or(0),
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
