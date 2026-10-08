//! The tool calls one subagent dispatch made, read back from the session's
//! completed history (Issue 276).
//!
//! A subagent-backed response returned no tool uses, so a caller that keeps
//! the session's trace (a raw provider outcome has no agent report to fall
//! back on) recorded a session of 80 inspection calls as having made none.
//! The history the runner captures holds every assistant `tool_use` block
//! and the `tool_result` that answered it; only the call's name, its input
//! and whether the result was an error are kept, never the tool's output.
//!
//! The trace is persisted, so it is bounded: at most [`MAX_TRACE_CALLS`]
//! calls, each input at most [`MAX_INPUT_BYTES`] of JSON. Nothing is cut
//! silently: a cut input is marked on its call, and the trace ends with one
//! summary entry named [`TOOL_TRACE_SUMMARY_NAME`] that counts every call,
//! the calls kept, the calls dropped and the inputs cut. The summary is also
//! what tells a reader the trace was captured at all, so a session that made
//! no call is told apart from one whose calls were not recorded.
use crate::runner::ToolUseEntry;
use archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The most calls one trace keeps. Later calls are counted, not kept.
pub(super) const MAX_TRACE_CALLS: usize = 400;
/// The most bytes of serialized JSON one kept input may hold.
pub(super) const MAX_INPUT_BYTES: usize = 4096;
/// The most top-level fields a cut input keeps.
const MAX_INPUT_FIELDS: usize = 8;
/// The most bytes a cut input keeps of one field name.
const MAX_KEY_BYTES: usize = 64;
/// The most bytes a cut input keeps of one field value.
const MAX_VALUE_BYTES: usize = 384;

/// Every `tool_use` block in `messages`, in call order, bounded as the
/// module says, followed by the trace summary. `output` is
/// `{"is_error": bool}` when a `tool_result` answered the call, and has no
/// `is_error` when none did (the session ended before the tool returned).
///
/// Returns nothing when `messages` holds no assistant message: then the
/// history was not captured for this dispatch, and an empty trace with a
/// summary would claim the session made no call.
pub(super) fn tool_uses(messages: &[Value]) -> Vec<ToolUseEntry> {
    if !messages
        .iter()
        .any(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
    {
        return Vec::new();
    }
    let blocks = || {
        messages
            .iter()
            .filter_map(|message| message.get("content").and_then(Value::as_array))
            .flatten()
    };
    let errors: HashMap<&str, bool> = blocks()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| {
            let id = block.get("tool_use_id").and_then(Value::as_str)?;
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some((id, is_error))
        })
        .collect();
    let mut calls = 0usize;
    let mut inputs_cut = 0usize;
    let mut entries = Vec::new();
    for block in blocks().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")) {
        calls += 1;
        if entries.len() == MAX_TRACE_CALLS {
            continue;
        }
        let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
        let mut output = Map::new();
        if let Some(is_error) = errors.get(id) {
            output.insert("is_error".into(), json!(is_error));
        }
        let input = match bounded_input(block.get("input").unwrap_or(&Value::Null)) {
            Ok(input) => input,
            Err((input, original_bytes)) => {
                inputs_cut += 1;
                output.insert("input_truncated".into(), json!(true));
                output.insert("input_bytes".into(), json!(original_bytes));
                input
            }
        };
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        entries.push(ToolUseEntry {
            tool_name: clip(name, MAX_KEY_BYTES),
            input,
            output: if output.is_empty() {
                Value::Null
            } else {
                Value::Object(output)
            },
        });
    }
    let kept = entries.len();
    entries.push(ToolUseEntry {
        tool_name: TOOL_TRACE_SUMMARY_NAME.to_string(),
        input: json!({
            "calls": calls,
            "kept": kept,
            "dropped": calls - kept,
            "inputs_truncated": inputs_cut,
            "max_calls": MAX_TRACE_CALLS,
            "max_input_bytes": MAX_INPUT_BYTES,
        }),
        output: Value::Null,
    });
    entries
}

/// The input itself when it fits; otherwise a cut copy and the original
/// size. The cut copy keeps the first fields, each clipped, and is never
/// larger than [`MAX_INPUT_BYTES`].
fn bounded_input(input: &Value) -> Result<Value, (Value, usize)> {
    let size = serde_json::to_string(input).map_or(usize::MAX, |text| text.len());
    if size <= MAX_INPUT_BYTES {
        return Ok(input.clone());
    }
    let cut = match input {
        Value::Object(map) => Value::Object(
            map.iter()
                .take(MAX_INPUT_FIELDS)
                .map(|(key, value)| (clip(key, MAX_KEY_BYTES), clipped_value(value)))
                .collect(),
        ),
        other => clipped_value(other),
    };
    let fits = serde_json::to_string(&cut).is_ok_and(|text| text.len() <= MAX_INPUT_BYTES);
    Err((if fits { cut } else { Value::Null }, size))
}

fn clipped_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(clip(text, MAX_VALUE_BYTES)),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        other => Value::String(clip(&other.to_string(), MAX_VALUE_BYTES)),
    }
}

/// At most `max` bytes of `text`, cut on a character boundary and ended with
/// an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max - '\u{2026}'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\u{2026}", &text[..end])
}

#[cfg(test)]
#[path = "tool_trace_tests.rs"]
mod tests;
