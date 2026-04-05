use crate::agent::AgentEvent;

// ---------------------------------------------------------------------------
// OutputFormat enum
// ---------------------------------------------------------------------------

/// Output format for print mode results.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputFormat {
    /// Plain text output (assistant text to stdout).
    Text,
    /// Single JSON object with content, usage, and cost.
    Json,
    /// Newline-delimited JSON stream events.
    StreamJson,
}

impl OutputFormat {
    /// Parse an output format string.
    ///
    /// Accepted values: `"text"`, `"json"`, `"stream-json"`.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "stream-json" => Ok(Self::StreamJson),
            other => Err(format!(
                "unknown output format '{other}': expected text, json, or stream-json"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a final result as a JSON string containing content, usage, and cost.
pub fn format_json_result(content: &str, usage: &archon_llm::types::Usage, cost: f64) -> String {
    let result = serde_json::json!({
        "content": content,
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
        },
        "cost": cost,
    });
    serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string())
}

/// Format a stream event as a single NDJSON line (terminated by `\n`).
///
/// Returns `{"type":"<event_type>","content":<payload>}\n`.
pub fn format_stream_event(event_type: &str, payload: &serde_json::Value) -> String {
    let event = serde_json::json!({
        "type": event_type,
        "content": payload,
    });
    let mut line = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
    line.push('\n');
    line
}

/// Format an `AgentEvent` based on the requested output format.
///
/// Returns `None` when the event should not produce output in the given format
/// (e.g. text deltas in `Json` mode are accumulated elsewhere, not emitted
/// per-delta).
pub fn format_agent_event(event: &AgentEvent, format: &OutputFormat) -> Option<String> {
    match format {
        OutputFormat::Text => format_agent_event_text(event),
        OutputFormat::Json => None, // Json mode accumulates; final result is emitted separately
        OutputFormat::StreamJson => format_agent_event_stream_json(event),
    }
}

/// Text mode: only emit text deltas directly.
fn format_agent_event_text(event: &AgentEvent) -> Option<String> {
    match event {
        AgentEvent::TextDelta(text) => Some(text.clone()),
        _ => None,
    }
}

/// StreamJson mode: emit each meaningful event as an NDJSON line.
fn format_agent_event_stream_json(event: &AgentEvent) -> Option<String> {
    match event {
        AgentEvent::TextDelta(text) => Some(format_stream_event(
            "text",
            &serde_json::json!({"text": text}),
        )),

        AgentEvent::ThinkingDelta(text) => Some(format_stream_event(
            "thinking",
            &serde_json::json!({"text": text}),
        )),

        AgentEvent::ToolCallStarted { name, id } => Some(format_stream_event(
            "tool_use_start",
            &serde_json::json!({"name": name, "id": id}),
        )),

        AgentEvent::ToolCallComplete { name, id, result } => Some(format_stream_event(
            "tool_result",
            &serde_json::json!({
                "name": name,
                "id": id,
                "content": result.content,
                "is_error": result.is_error,
            }),
        )),

        AgentEvent::TurnComplete {
            input_tokens,
            output_tokens,
        } => Some(format_stream_event(
            "turn_complete",
            &serde_json::json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            }),
        )),

        AgentEvent::Error(msg) => Some(format_stream_event(
            "error",
            &serde_json::json!({"message": msg}),
        )),

        AgentEvent::ApiCallStarted { model } => Some(format_stream_event(
            "api_call",
            &serde_json::json!({"model": model}),
        )),

        AgentEvent::CompactionTriggered => {
            Some(format_stream_event("compaction", &serde_json::json!({})))
        }

        // Events that don't produce stream output
        AgentEvent::UserPromptReady
        | AgentEvent::PermissionRequired { .. }
        | AgentEvent::PermissionGranted { .. }
        | AgentEvent::PermissionDenied { .. }
        | AgentEvent::SessionComplete => None,
    }
}
