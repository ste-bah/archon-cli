use serde::Deserialize;

use crate::types::{ContentBlockType, Usage};

// ---------------------------------------------------------------------------
// Stream events -- typed enum for all SSE event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum StreamEvent {
    MessageStart {
        id: String,
        model: String,
        usage: Usage,
    },
    ContentBlockStart {
        index: u32,
        block_type: ContentBlockType,
        // For tool_use blocks
        tool_use_id: Option<String>,
        tool_name: Option<String>,
    },
    TextDelta {
        index: u32,
        text: String,
    },
    ThinkingDelta {
        index: u32,
        thinking: String,
    },
    InputJsonDelta {
        index: u32,
        partial_json: String,
    },
    SignatureDelta {
        index: u32,
        signature: String,
    },
    ContentBlockStop {
        index: u32,
    },
    MessageDelta {
        stop_reason: Option<String>,
        usage: Option<Usage>,
    },
    MessageStop,
    Ping,
    Error {
        error_type: String,
        message: String,
    },
}

// ---------------------------------------------------------------------------
// SSE line parser
// ---------------------------------------------------------------------------

/// Parse a raw SSE data line into a StreamEvent.
///
/// SSE format:
/// ```text
/// event: message_start
/// data: {"type":"message_start","message":{"id":"msg_xxx",...}}
/// ```
pub fn parse_sse_event(event_type: &str, data: &str) -> Result<StreamEvent, StreamError> {
    match event_type {
        "message_start" => parse_message_start(data),
        "content_block_start" => parse_content_block_start(data),
        "content_block_delta" => parse_content_block_delta(data),
        "content_block_stop" => parse_content_block_stop(data),
        "message_delta" => parse_message_delta(data),
        "message_stop" => Ok(StreamEvent::MessageStop),
        "ping" => Ok(StreamEvent::Ping),
        "error" => parse_error(data),
        other => Err(StreamError::UnknownEvent(other.to_string())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("failed to parse SSE data: {0}")]
    ParseError(String),

    #[error("unknown SSE event type: {0}")]
    UnknownEvent(String),
}

// ---------------------------------------------------------------------------
// Event parsers
// ---------------------------------------------------------------------------

fn parse_message_start(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct Outer {
        message: MessageStartData,
    }
    #[derive(Deserialize)]
    struct MessageStartData {
        id: String,
        model: String,
        #[serde(default)]
        usage: Usage,
    }

    let outer: Outer = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("message_start: {e}")))?;

    Ok(StreamEvent::MessageStart {
        id: outer.message.id,
        model: outer.message.model,
        usage: outer.message.usage,
    })
}

fn parse_content_block_start(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct Outer {
        index: u32,
        content_block: ContentBlockData,
    }
    #[derive(Deserialize)]
    struct ContentBlockData {
        #[serde(rename = "type")]
        block_type: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
    }

    let outer: Outer = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("content_block_start: {e}")))?;

    let block_type = match outer.content_block.block_type.as_str() {
        "text" => ContentBlockType::Text,
        "thinking" => ContentBlockType::Thinking,
        "tool_use" => ContentBlockType::ToolUse,
        other => {
            return Err(StreamError::ParseError(format!(
                "unknown content block type: {other}"
            )))
        }
    };

    Ok(StreamEvent::ContentBlockStart {
        index: outer.index,
        block_type,
        tool_use_id: outer.content_block.id,
        tool_name: outer.content_block.name,
    })
}

fn parse_content_block_delta(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct Outer {
        index: u32,
        delta: DeltaData,
    }
    #[derive(Deserialize)]
    struct DeltaData {
        #[serde(rename = "type")]
        delta_type: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        thinking: Option<String>,
        #[serde(default)]
        partial_json: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    }

    let outer: Outer = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("content_block_delta: {e}")))?;

    match outer.delta.delta_type.as_str() {
        "text_delta" => Ok(StreamEvent::TextDelta {
            index: outer.index,
            text: outer.delta.text.unwrap_or_default(),
        }),
        "thinking_delta" => Ok(StreamEvent::ThinkingDelta {
            index: outer.index,
            thinking: outer.delta.thinking.unwrap_or_default(),
        }),
        "input_json_delta" => Ok(StreamEvent::InputJsonDelta {
            index: outer.index,
            partial_json: outer.delta.partial_json.unwrap_or_default(),
        }),
        "signature_delta" => Ok(StreamEvent::SignatureDelta {
            index: outer.index,
            signature: outer.delta.signature.unwrap_or_default(),
        }),
        other => Err(StreamError::ParseError(format!(
            "unknown delta type: {other}"
        ))),
    }
}

fn parse_content_block_stop(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct Outer {
        index: u32,
    }

    let outer: Outer = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("content_block_stop: {e}")))?;

    Ok(StreamEvent::ContentBlockStop {
        index: outer.index,
    })
}

fn parse_message_delta(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct Outer {
        delta: DeltaFields,
        #[serde(default)]
        usage: Option<Usage>,
    }
    #[derive(Deserialize)]
    struct DeltaFields {
        stop_reason: Option<String>,
    }

    let outer: Outer = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("message_delta: {e}")))?;

    Ok(StreamEvent::MessageDelta {
        stop_reason: outer.delta.stop_reason,
        usage: outer.usage,
    })
}

fn parse_error(data: &str) -> Result<StreamEvent, StreamError> {
    #[derive(Deserialize)]
    struct ErrorData {
        error: ErrorFields,
    }
    #[derive(Deserialize)]
    struct ErrorFields {
        #[serde(rename = "type")]
        error_type: String,
        message: String,
    }

    let outer: ErrorData = serde_json::from_str(data)
        .map_err(|e| StreamError::ParseError(format!("error event: {e}")))?;

    Ok(StreamEvent::Error {
        error_type: outer.error.error_type,
        message: outer.error.message,
    })
}

// ---------------------------------------------------------------------------
// Raw SSE line splitter
// ---------------------------------------------------------------------------

/// Parse raw SSE text into (event_type, data) pairs.
///
/// SSE lines:
/// ```text
/// event: message_start
/// data: {"type":"message_start",...}
///
/// event: content_block_delta
/// data: {"type":"content_block_delta",...}
/// ```
pub fn split_sse_lines(raw: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    let mut current_event = "";

    for line in raw.lines() {
        if let Some(event) = line.strip_prefix("event: ") {
            current_event = event.trim();
        } else if let Some(data) = line.strip_prefix("data: ") {
            if !current_event.is_empty() {
                pairs.push((current_event, data.trim()));
                current_event = "";
            }
        }
    }

    pairs
}
