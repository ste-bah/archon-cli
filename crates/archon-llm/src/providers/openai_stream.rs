//! OpenAI-format SSE chunk parsing, shared by `OpenAiProvider`,
//! `LocalProvider`, and every `OpenAiCompatProvider` backend via
//! `stream_decode`.
//!
//! Split out of `providers::openai` in #123: adding reasoning-delta handling
//! pushed that file past the 500-line gate, and the parsing half is a
//! self-contained unit with a different reason to change than the provider
//! itself.

use crate::providers::openai_protocol::{usage_event, usage_from_openai_chunk};
use crate::streaming::StreamEvent;
use crate::types::ContentBlockType;

/// Parse a single OpenAI SSE JSON chunk into StreamEvents.
///
/// Handles text deltas, tool call starts/argument chunks, and finish reasons.
pub(crate) fn parse_openai_sse_chunk(chunk: &str) -> Vec<StreamEvent> {
    let value: serde_json::Value = match serde_json::from_str(chunk) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let usage = usage_from_openai_chunk(&value);
    let choices = match value.get("choices").and_then(|c| c.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return usage.map_or_else(Vec::new, usage_event),
    };

    let choice = &choices[0];
    let delta = match choice.get("delta") {
        Some(d) => d,
        None => return usage.map_or_else(Vec::new, usage_event),
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|fr| fr.as_str())
        .unwrap_or("");

    let mut events = Vec::new();

    // Reasoning delta (#123). vLLM's `--reasoning-parser` splits the model's
    // thinking out of `content` into its own field, so without this branch the
    // tokens are paid for and silently dropped.
    //
    // The spelling is NOT stable: a live vLLM 0.25 server hosting
    // DeepSeek-V4-Flash emits `reasoning`, while other vLLM builds and parsers
    // emit `reasoning_content`. Accept either rather than betting on one.
    //
    // Emitted BEFORE the text delta because a single chunk can carry both keys
    // — the transition chunk where thinking ends and the answer begins does
    // exactly that on the observed server — and treating them as mutually
    // exclusive would drop a token.
    if let Some(reasoning) = delta
        .get("reasoning")
        .or_else(|| delta.get("reasoning_content"))
        .and_then(|r| r.as_str())
        && !reasoning.is_empty()
    {
        events.push(StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
            tool_use_id: None,
            tool_name: None,
        });
        events.push(StreamEvent::ThinkingDelta {
            index: 0,
            thinking: reasoning.to_string(),
        });
    }

    // Text content delta.
    if let Some(content) = delta.get("content").and_then(|c| c.as_str())
        && !content.is_empty()
    {
        events.push(StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        });
        events.push(StreamEvent::TextDelta {
            index: 0,
            text: content.to_string(),
        });
    }

    // Tool call deltas.
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
        for tc in tool_calls {
            let tc_index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let tc_id = tc.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
            let func = tc.get("function");

            let func_name = func
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            let func_args = func
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());

            // If we have an id and name, this is the start of a new tool call.
            if tc_id.is_some() && func_name.is_some() {
                events.push(StreamEvent::ContentBlockStart {
                    index: tc_index,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: tc_id,
                    tool_name: func_name,
                });
            }

            // Argument chunk.
            if let Some(args) = func_args
                && !args.is_empty()
            {
                events.push(StreamEvent::InputJsonDelta {
                    index: tc_index,
                    partial_json: args,
                });
            }
        }
    }

    // Finish reason handling.
    match finish_reason {
        "tool_calls" => {
            events.push(StreamEvent::ContentBlockStop { index: 0 });
            events.push(StreamEvent::MessageDelta {
                stop_reason: Some("tool_use".to_string()),
                usage: None,
            });
        }
        "stop" => {
            events.push(StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".to_string()),
                usage: None,
            });
        }
        _ => {}
    }

    if let Some(usage) = usage {
        events.push(StreamEvent::MessageDelta {
            stop_reason: None,
            usage: Some(usage),
        });
    }
    events
}
