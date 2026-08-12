//! Request-boundary projections of the message history (#75 A1).
//!
//! Neither projection mutates the canonical history: the stored transcript
//! keeps full fidelity, and only the copy handed to the provider is trimmed.
//! Both are the round's one O(N) pass over the transcript (#171 part 2), so
//! they are written to copy as little as they can get away with.

use super::{ContextToolOutput, cap_tool_output_for_context, cap_tool_output_to_bytes};

/// Cap every tool result — recent ones included — to a single-field byte
/// budget, for the last rung of the #103 recovery ladder.
pub(crate) fn project_messages_for_emergency_retry(
    messages: &[serde_json::Value],
    max_serialized_field_bytes: usize,
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|message| {
            project_message(message, |_, content| {
                trimmed(cap_tool_output_to_bytes(
                    content,
                    max_serialized_field_bytes,
                ))
            })
        })
        .collect()
}

/// Trim tool results outside the preserved recent turns to their per-tool
/// context budget.
pub(crate) fn project_messages_for_request(
    messages: &[serde_json::Value],
    preserve_recent_turns: u32,
) -> Vec<serde_json::Value> {
    let preserve_from = recent_turn_start(messages, preserve_recent_turns);
    let tool_names = tool_names_by_id(messages);

    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if index >= preserve_from {
                return message.clone();
            }
            project_message(message, |block, content| {
                let tool_name = block
                    .get("tool_use_id")
                    .and_then(|value| value.as_str())
                    .and_then(|id| tool_names.get(id))
                    .map(String::as_str)
                    .unwrap_or("");
                trimmed(cap_tool_output_for_context(tool_name, content))
            })
        })
        .collect()
}

fn trimmed(output: ContextToolOutput) -> Option<String> {
    output.truncated.then_some(output.content)
}

/// Copy one message, replacing the `content` of every `tool_result` block that
/// `trim` rewrites.
///
/// The projection used to clone the whole array up front and then overwrite
/// the trimmed fields, so every byte trimming was about to discard got copied
/// on the way out first — the largest allocation in request prep, spent on the
/// content guaranteed not to survive it (#171 part 2). Here a message that
/// needs no trimming is cloned as-is, and a message that does is rebuilt
/// around the replacement string, so the discarded content is never copied.
///
/// The rebuild walks the existing keys in order and substitutes only
/// `content`, which is what keeps the serialized bytes identical to the
/// in-place overwrite it replaces — and #75 A2's cache-marker placement is
/// position-sensitive, so that is load-bearing rather than incidental.
fn project_message(
    message: &serde_json::Value,
    mut trim: impl FnMut(&serde_json::Value, &str) -> Option<String>,
) -> serde_json::Value {
    let Some(blocks) = message.get("content").and_then(|value| value.as_array()) else {
        return message.clone();
    };
    let mut replacements: Vec<Option<String>> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let Some(replacement) = trimmable_content(block).and_then(|content| trim(block, content))
        else {
            continue;
        };
        if replacements.is_empty() {
            replacements.resize_with(blocks.len(), || None);
        }
        replacements[index] = Some(replacement);
    }
    if replacements.is_empty() {
        return message.clone();
    }
    let projected: Vec<serde_json::Value> = blocks
        .iter()
        .zip(replacements)
        .map(|(block, replacement)| match replacement {
            None => block.clone(),
            Some(content) => with_content(block, serde_json::Value::String(content)),
        })
        .collect();
    with_content(message, serde_json::Value::Array(projected))
}

fn trimmable_content(block: &serde_json::Value) -> Option<&str> {
    if block.get("type").and_then(|value| value.as_str()) != Some("tool_result") {
        return None;
    }
    block.get("content").and_then(|value| value.as_str())
}

/// Copy a JSON object's entries in their existing order, swapping in a new
/// `content` value.
fn with_content(value: &serde_json::Value, content: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut content = Some(content);
    serde_json::Value::Object(
        object
            .iter()
            .map(|(key, existing)| {
                let entry = (key == "content")
                    .then(|| content.take())
                    .flatten()
                    .unwrap_or_else(|| existing.clone());
                (key.clone(), entry)
            })
            .collect(),
    )
}

fn recent_turn_start(messages: &[serde_json::Value], preserve_recent_turns: u32) -> usize {
    if preserve_recent_turns == 0 {
        return messages.len();
    }
    let mut turns = 0;
    for (index, message) in messages.iter().enumerate().rev() {
        if is_user_prompt(message) {
            turns += 1;
            if turns == preserve_recent_turns {
                return index;
            }
        }
    }
    0
}

fn is_user_prompt(message: &serde_json::Value) -> bool {
    if message.get("role").and_then(|value| value.as_str()) != Some("user") {
        return false;
    }
    match message.get("content") {
        Some(serde_json::Value::Array(blocks)) => {
            !blocks.is_empty()
                && !blocks.iter().all(|block| {
                    block.get("type").and_then(|value| value.as_str()) == Some("tool_result")
                })
        }
        Some(_) => true,
        None => false,
    }
}

fn tool_names_by_id(messages: &[serde_json::Value]) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    for block in messages
        .iter()
        .filter_map(|message| message.get("content").and_then(|value| value.as_array()))
        .flatten()
    {
        if block.get("type").and_then(|value| value.as_str()) == Some("tool_use")
            && let (Some(id), Some(name)) = (
                block.get("id").and_then(|value| value.as_str()),
                block.get("name").and_then(|value| value.as_str()),
            )
        {
            names.insert(id.to_string(), name.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_tool_results_are_trimmed_but_recent_turns_remain_exact() {
        let old_content = format!("{}{}", "old-head".repeat(10_000), "old-tail".repeat(10_000));
        let recent_content = "recent-result".repeat(10_000);
        let messages = vec![
            serde_json::json!({"role": "user", "content": "first turn"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "old-tool", "name": "Bash", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "old-tool",
                "content": old_content, "is_error": false
            }]}),
            serde_json::json!({"role": "user", "content": "second turn"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "recent-tool", "name": "Read", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "recent-tool",
                "content": recent_content, "is_error": true
            }]}),
        ];

        let projected = project_messages_for_request(&messages, 1);

        let old = projected[2]["content"][0]["content"]
            .as_str()
            .expect("projected old tool result");
        assert!(old.contains("tool output"));
        assert!(old.len() < messages[2]["content"][0]["content"].as_str().unwrap().len());
        assert_eq!(
            projected[5]["content"][0]["content"],
            messages[5]["content"][0]["content"]
        );
        assert_eq!(projected[5]["content"][0]["is_error"], true);
        assert_eq!(projected[5]["content"][0]["tool_use_id"], "recent-tool");
    }

    /// A trimmed block is rebuilt rather than overwritten (#171 part 2), so the
    /// serialized result has to match the overwrite byte for byte — key order
    /// included, because Anthropic cache markers are position-sensitive.
    #[test]
    fn a_trimmed_block_serializes_exactly_as_an_in_place_overwrite_would() {
        let messages = vec![
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "old-tool", "name": "Bash", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "old-tool",
                "content": "x".repeat(100_000), "is_error": false, "trailing": "kept"
            }]}),
            serde_json::json!({"role": "user", "content": "recent turn"}),
        ];

        let projected = project_messages_for_request(&messages, 1);

        let mut overwritten = messages.clone();
        let trimmed = projected[1]["content"][0]["content"].clone();
        overwritten[1]["content"][0]["content"] = trimmed;
        assert_eq!(
            serde_json::to_string(&projected).expect("serialize projection"),
            serde_json::to_string(&overwritten).expect("serialize overwrite")
        );
    }

    #[test]
    fn emergency_projection_caps_recent_tool_results_without_mutating_canonical_history() {
        let recent_content = format!("HEAD{}TAIL", "é".repeat(100_000));
        let messages = vec![
            serde_json::json!({"role": "user", "content": "inspect"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "recent-tool", "name": "Read", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "recent-tool",
                "content": recent_content, "is_error": false
            }]}),
        ];
        let original = messages.clone();

        let projected = project_messages_for_emergency_retry(&messages, 4_096);

        let content = projected[2]["content"][0]["content"]
            .as_str()
            .expect("projected recent tool result");
        assert!(
            serde_json::to_vec(&serde_json::Value::String(content.to_string()))
                .expect("serialize provider field")
                .len()
                <= 4_096
        );
        assert!(content.starts_with("HEAD"));
        assert!(content.ends_with("TAIL"));
        assert!(content.contains("omitted"));
        assert_eq!(messages, original);
    }

    #[test]
    fn emergency_projection_leaves_small_recent_results_byte_identical() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tool-1",
                "content": "small output",
                "is_error": false
            }]
        })];

        assert_eq!(
            project_messages_for_emergency_retry(&messages, 4_096),
            messages
        );
    }

    #[test]
    fn request_projection_does_not_mutate_canonical_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "first turn"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "tool-1", "name": "Read", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "tool-1",
                "content": "x".repeat(100_000), "is_error": false
            }]}),
            serde_json::json!({"role": "user", "content": "second turn"}),
        ];
        let original = messages.clone();

        let _ = project_messages_for_request(&messages, 1);

        assert_eq!(messages, original);
    }
}
