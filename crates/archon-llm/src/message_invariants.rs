//! Shared message-shape repair for Anthropic Messages compatible providers.
//!
//! Anthropic, Bedrock Claude, and Vertex Claude require strict adjacency:
//! every `tool_result` must immediately follow an assistant `tool_use`, and
//! every assistant `tool_use` must be covered by the next user message's
//! `tool_result` blocks. Codex/OpenAI use a different tool schema
//! (`tool_calls`, `role: "tool"`, `tool_call_id`) and need a separate
//! sanitizer if pairing breakage is observed there.

use serde_json::Value;

pub fn sanitize_anthropic_shape(messages: Vec<Value>) -> Vec<Value> {
    let mut sanitized = Vec::with_capacity(messages.len());
    for mut message in messages {
        normalize_message_role(&mut message);
        if has_tool_result(&message)
            && !previous_assistant_has_tool_uses(sanitized.last(), &message)
        {
            message = orphan_tool_result_as_text(&message);
        }
        sanitized.push(message);
    }

    for i in 0..sanitized.len() {
        let next_result_ids = sanitized
            .get(i + 1)
            .map(tool_result_ids_owned)
            .unwrap_or_default();
        if has_orphan_tool_use(&sanitized[i], &next_result_ids) {
            demote_orphan_tool_uses_to_text(&mut sanitized[i], &next_result_ids);
        }
    }

    sanitized
}

fn normalize_message_role(message: &mut Value) {
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user");
    if !matches!(role, "user" | "assistant") {
        message["role"] = Value::String("user".into());
    }
}

fn has_tool_result(message: &Value) -> bool {
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
}

fn previous_assistant_has_tool_uses(previous: Option<&Value>, result_message: &Value) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return false;
    }
    let result_ids = tool_result_ids(result_message);
    !result_ids.is_empty()
        && result_ids
            .iter()
            .all(|id| assistant_has_tool_use(previous, id))
}

fn tool_result_ids(message: &Value) -> Vec<&str> {
    if message.get("role").and_then(|v| v.as_str()) != Some("user") {
        return Vec::new();
    }
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
        .filter_map(|block| block.get("tool_use_id").and_then(|v| v.as_str()))
        .collect()
}

fn tool_result_ids_owned(message: &Value) -> Vec<String> {
    tool_result_ids(message)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn assistant_has_tool_use(message: &Value, id: &str) -> bool {
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|block| {
            block.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                && block.get("id").and_then(|v| v.as_str()) == Some(id)
        })
}

fn orphan_tool_result_as_text(message: &Value) -> Value {
    let text = message
        .get("content")
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .map(tool_result_block_text)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| {
            message
                .get("content")
                .cloned()
                .unwrap_or_default()
                .to_string()
        });
    serde_json::json!({ "role": "user", "content": text })
}

fn tool_result_block_text(block: &Value) -> String {
    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
        let id = block
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = block
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                block
                    .get("content")
                    .cloned()
                    .unwrap_or_default()
                    .to_string()
            });
        return format!("[Tool result {id}] {content}");
    }
    block
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| block.to_string())
}

fn has_orphan_tool_use(message: &Value, next_result_ids: &[String]) -> bool {
    if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return false;
    }
    let Some(blocks) = message.get("content").and_then(|v| v.as_array()) else {
        return false;
    };
    blocks.iter().any(|block| {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            return false;
        }
        match block.get("id").and_then(|v| v.as_str()) {
            Some(id) => !next_result_ids.iter().any(|result_id| result_id == id),
            None => true,
        }
    })
}

fn demote_orphan_tool_uses_to_text(message: &mut Value, next_result_ids: &[String]) {
    let Some(blocks) = message.get_mut("content").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for block in blocks.iter_mut() {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            continue;
        }
        let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if next_result_ids.iter().any(|result_id| result_id == id) {
            continue;
        }
        let name = block
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        *block = serde_json::json!({
            "type": "text",
            "text": format!(
                "[tool call interrupted: {name} (id={id}) - no result available]"
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_converts_system_messages_to_user() {
        let messages = sanitize_anthropic_shape(vec![serde_json::json!({
            "role": "system",
            "content": "boundary"
        })]);

        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "boundary");
    }

    #[test]
    fn sanitizer_textifies_orphan_tool_result() {
        let messages = sanitize_anthropic_shape(vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tool-1",
                "content": "failed"
            }]
        })]);

        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0]["content"].as_str().unwrap().contains("tool-1"));
        assert!(messages[0]["content"].as_str().unwrap().contains("failed"));
    }

    #[test]
    fn sanitizer_textifies_orphan_tool_use() {
        let messages = sanitize_anthropic_shape(vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tool-1",
                "name": "Read",
                "input": {}
            }]
        })]);

        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert!(
            messages[0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("tool-1")
        );
    }

    #[test]
    fn sanitizer_preserves_valid_tool_pair() {
        let messages = sanitize_anthropic_shape(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}]
            }),
        ]);

        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn sanitizer_only_demotes_uncovered_tool_uses() {
        let messages = sanitize_anthropic_shape(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "tool-2", "name": "Write", "input": {}}
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}]
            }),
        ]);

        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[0]["content"][1]["type"], "text");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn sanitizer_requires_immediate_tool_result_pair() {
        let messages = sanitize_anthropic_shape(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({"role": "user", "content": "not a tool result"}),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "late"}]
            }),
        ]);

        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert!(messages[2]["content"].as_str().unwrap().contains("tool-1"));
    }
}
