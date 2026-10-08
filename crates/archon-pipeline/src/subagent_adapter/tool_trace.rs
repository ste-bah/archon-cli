//! The tool calls one subagent dispatch made, read back from the session's
//! completed history (Issue 276).
//!
//! A subagent-backed response returned no tool uses, so a caller that keeps
//! the session's trace (a raw provider outcome has no agent report to fall
//! back on) recorded a session of 80 inspection calls as having made none.
//! The history the runner captures holds every assistant `tool_use` block
//! and the `tool_result` that answered it; only the call's name, its input
//! and whether the result was an error are kept, never the tool's output.
use crate::runner::ToolUseEntry;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Every `tool_use` block in `messages`, in call order. `output` is
/// `{"is_error": bool}` when a `tool_result` answered the call, and null when
/// none did (the session ended before the tool returned).
pub(super) fn tool_uses(messages: &[Value]) -> Vec<ToolUseEntry> {
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
    blocks()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|block| {
            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
            ToolUseEntry {
                tool_name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input: block.get("input").cloned().unwrap_or(Value::Null),
                output: errors
                    .get(id)
                    .map(|is_error| json!({ "is_error": is_error }))
                    .unwrap_or(Value::Null),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_each_call_with_its_result_and_keeps_no_tool_output() {
        let messages = vec![
            json!({"role": "user", "content": "task"}),
            json!({"role": "assistant", "content": [
                {"type": "text", "text": "looking"},
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "false"}},
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "SECRET BODY", "is_error": false},
                {"type": "tool_result", "tool_use_id": "t2", "content": "", "is_error": true},
            ]}),
            json!({"role": "assistant", "content": [
                {"type": "tool_use", "id": "t3", "name": "Grep", "input": {"pattern": "x"}},
            ]}),
        ];
        let uses = tool_uses(&messages);
        let names: Vec<_> = uses.iter().map(|u| u.tool_name.as_str()).collect();
        assert_eq!(names, ["Read", "Bash", "Grep"]);
        assert_eq!(uses[0].input["file_path"], "a.rs");
        assert_eq!(uses[0].output, json!({"is_error": false}));
        assert_eq!(uses[1].output, json!({"is_error": true}));
        assert_eq!(uses[2].output, Value::Null);
        assert!(
            !serde_json::to_string(&uses[0].output)
                .unwrap()
                .contains("SECRET")
        );
    }
}
