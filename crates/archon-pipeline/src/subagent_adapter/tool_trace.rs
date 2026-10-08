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
//! The trace is persisted, so each input is reduced to the allow-list of
//! [`archon_tools::tool_trace_input`]: what a call touched (paths, a URL
//! without its query, a Bash command with credential values replaced),
//! never its content, each value redacted before it is cut to
//! [`MAX_VALUE_BYTES`]. At most [`MAX_TRACE_CALLS`] calls are kept. Nothing
//! is cut silently: a cut or reduced input is marked on its call, and the
//! trace ends with one summary entry named [`TOOL_TRACE_SUMMARY_NAME`] that
//! counts every call, the calls kept, the calls dropped and the inputs cut.
//! The summary is also what tells a reader the trace was captured at all,
//! so a session that made no call is told apart from one whose calls were
//! not recorded.
use crate::runner::ToolUseEntry;
use archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
use archon_tools::tool_trace_input::{clip, safe_input};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The most calls one trace keeps. Later calls are counted, not kept.
pub(super) const MAX_TRACE_CALLS: usize = 400;
/// The most bytes one kept input value may hold. With the allow-list this
/// bounds a kept input to a few KiB.
pub(super) const MAX_VALUE_BYTES: usize = 384;
/// The most bytes of a call's tool name kept.
const MAX_NAME_BYTES: usize = 64;

/// Every `tool_use` block in `messages`, in call order, bounded as the
/// module says, followed by the trace summary. `output` is
/// `{"is_error": bool}` when a `tool_result` answered the call, and has no
/// `is_error` when none did (the session ended before the tool returned);
/// `input_truncated` and `input_keys_dropped` mark a reduced input.
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
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut output = Map::new();
        if let Some(is_error) = errors.get(id) {
            output.insert("is_error".into(), json!(is_error));
        }
        let safe = safe_input(
            name,
            block.get("input").unwrap_or(&Value::Null),
            MAX_VALUE_BYTES,
        );
        if safe.cut {
            inputs_cut += 1;
            output.insert("input_truncated".into(), json!(true));
        }
        if safe.dropped_keys > 0 {
            output.insert("input_keys_dropped".into(), json!(safe.dropped_keys));
        }
        entries.push(ToolUseEntry {
            tool_name: clip(name, MAX_NAME_BYTES),
            input: safe.input,
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
            "max_value_bytes": MAX_VALUE_BYTES,
        }),
        output: Value::Null,
    });
    entries
}

#[cfg(test)]
#[path = "tool_trace_tests.rs"]
mod tests;
