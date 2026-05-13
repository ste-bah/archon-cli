use archon_context::messages::ContextMessage;
use serde_json::Value;

pub(super) fn to_summary_context_messages(messages: &[Value]) -> Vec<ContextMessage> {
    messages
        .iter()
        .map(|message| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            let content = message.get("content").unwrap_or(&Value::Null);
            let text = textualize_content(content);
            ContextMessage {
                role,
                estimated_tokens: (text.len() as f64 / 4.0).ceil() as u64,
                content: Value::String(text),
            }
        })
        .collect()
}

pub(super) fn trim_oldest_safe_api_round(
    messages: &mut Vec<ContextMessage>,
    attempt: usize,
) -> bool {
    if attempt >= 2 || messages.len() <= 5 {
        return false;
    }
    let max_drop = messages.len().saturating_sub(4);
    let requested = (messages.len() / 4).max(1).min(max_drop);
    let Some(drop_count) = safe_api_round_drop_count(messages, requested, max_drop) else {
        return false;
    };
    messages.drain(0..drop_count);
    true
}

fn safe_api_round_drop_count(
    messages: &[ContextMessage],
    requested: usize,
    max_drop: usize,
) -> Option<usize> {
    (requested..=max_drop)
        .chain((1..requested).rev())
        .find(|&candidate| candidate > 0 && !starts_with_tool_result(messages, candidate))
}

fn starts_with_tool_result(messages: &[ContextMessage], index: usize) -> bool {
    messages
        .get(index)
        .is_some_and(|message| message.role == "user" && content_has_tool_result(&message.content))
}

fn content_has_tool_result(content: &Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

fn textualize_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .map(textualize_block)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => sanitize_json_for_summary(other).to_string(),
    }
}

fn textualize_block(block: &Value) -> String {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object");
    match kind {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "thinking" => block
            .get("thinking")
            .and_then(Value::as_str)
            .map(|text| format!("[thinking] {text}"))
            .unwrap_or_default(),
        "image" | "image_url" | "input_image" => {
            "[image] omitted from compaction summary input".to_string()
        }
        "document" | "file" | "input_file" => {
            let name = block
                .get("name")
                .or_else(|| block.get("filename"))
                .and_then(Value::as_str)
                .unwrap_or("unnamed");
            format!("[document] {name} omitted from compaction summary input")
        }
        "tool_use" => {
            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
            let id = block.get("id").and_then(Value::as_str).unwrap_or("unknown");
            let input = block.get("input").map(sanitize_json_for_summary);
            format!(
                "[tool_use id={id} name={name}] input={}",
                input.unwrap_or(Value::Null)
            )
        }
        "tool_result" => {
            let id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = block
                .get("content")
                .map(textualize_content)
                .unwrap_or_default();
            format!("[tool_result id={id}] {content}")
        }
        _ => sanitize_json_for_summary(block).to_string(),
    }
}

fn sanitize_json_for_summary(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in map {
                if matches!(key.as_str(), "data" | "base64" | "bytes" | "source") {
                    sanitized.insert(key.clone(), Value::String("[omitted]".into()));
                } else {
                    sanitized.insert(key.clone(), sanitize_json_for_summary(value));
                }
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_json_for_summary).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_blocks_are_textualized_without_payload_data() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "look at this"},
                {"type": "image", "source": {"type": "base64", "data": "abcdef"}},
                {"type": "document", "name": "notes.pdf", "source": {"data": "payload"}}
            ]
        })];

        let summary = to_summary_context_messages(&messages);
        let text = summary[0].content.as_str().expect("summary text");
        assert!(text.contains("[image]"));
        assert!(text.contains("[document] notes.pdf"));
        assert!(!text.contains("abcdef"));
        assert!(!text.contains("payload"));
    }

    #[test]
    fn summary_retry_trim_does_not_start_with_tool_result() {
        let mut messages = vec![
            ContextMessage::user("old 0"),
            ContextMessage::assistant("old 1"),
            ContextMessage {
                role: "assistant".into(),
                content: serde_json::json!([
                    {"type": "tool_use", "id": "tool-1", "name": "Bash", "input": {}}
                ]),
                estimated_tokens: 1,
            },
            ContextMessage {
                role: "user".into(),
                content: serde_json::json!([
                    {"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}
                ]),
                estimated_tokens: 1,
            },
            ContextMessage::assistant("recent 4"),
            ContextMessage::user("recent 5"),
            ContextMessage::assistant("recent 6"),
            ContextMessage::user("recent 7"),
        ];

        assert!(trim_oldest_safe_api_round(&mut messages, 0));
        assert!(!starts_with_tool_result(&messages, 0));
    }
}
