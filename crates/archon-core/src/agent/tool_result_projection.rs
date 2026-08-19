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
        // Emergency retry caps everything, so every block already passes
        // through `project_message` and is stripped there.
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
                // Untrimmed, but still stripped: archon's spill key must not
                // reach a provider from the preserved turns either.
                return strip_spill_keys(message);
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

/// Append the spill locator to a note that has just said bytes were omitted.
///
/// Saying "omitted 180000 bytes" and stopping there tells the model it has lost
/// something and gives it no way to get it back. The path turns the note into
/// an instruction it can act on (#189 Phase 1).
fn with_spill_note(block: &serde_json::Value, trimmed: Option<String>) -> Option<String> {
    let trimmed = trimmed?;
    let Some(path) = block
        .get(crate::agent::SPILL_PATH_KEY)
        .and_then(|value| value.as_str())
    else {
        return Some(trimmed);
    };
    Some(trimmed.replace(
        " before replaying tool output to the model.]",
        &format!(
            " before replaying tool output to the model. Full output: {path} (read it if you need the omitted region).]"
        ),
    ))
}

/// Remove archon's spill key without otherwise touching the message.
fn strip_spill_keys(message: &serde_json::Value) -> serde_json::Value {
    project_message(message, |_, _| None)
}

/// Strip archon's own bookkeeping from a block before it goes to a provider.
///
/// The spill path is recorded in history so it survives persistence and
/// forking, but it is not part of any provider's `tool_result` schema and has
/// no business on the wire.
fn without_spill_key(block: &serde_json::Value) -> Option<serde_json::Value> {
    let object = block.as_object()?;
    if !object.contains_key(crate::agent::SPILL_PATH_KEY) {
        return None;
    }
    let mut stripped = object.clone();
    stripped.remove(crate::agent::SPILL_PATH_KEY);
    Some(serde_json::Value::Object(stripped))
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
    let mut replacements: Vec<Option<serde_json::Value>> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let trimmed = trimmable_content(block).and_then(|content| trim(block, content));
        // A trimmed block gets the locator appended to its note; any block
        // carrying the locator gets it removed on the way out, trimmed or not.
        let rebuilt = match with_spill_note(block, trimmed) {
            Some(content) => {
                let base = without_spill_key(block).unwrap_or_else(|| block.clone());
                Some(with_content(&base, serde_json::Value::String(content)))
            }
            None => without_spill_key(block),
        };
        let Some(rebuilt) = rebuilt else {
            continue;
        };
        if replacements.is_empty() {
            replacements.resize_with(blocks.len(), || None);
        }
        replacements[index] = Some(rebuilt);
    }
    if replacements.is_empty() {
        return message.clone();
    }
    let projected: Vec<serde_json::Value> = blocks
        .iter()
        .zip(replacements)
        .map(|(block, replacement)| replacement.unwrap_or_else(|| block.clone()))
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

    fn spilled_history(path: &str) -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"role": "user", "content": "first turn"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "old-tool", "name": "Bash", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "old-tool",
                "content": "x".repeat(100_000), "is_error": false,
                crate::agent::SPILL_PATH_KEY: path
            }]}),
            serde_json::json!({"role": "user", "content": "second turn"}),
        ]
    }

    /// The note has to name the file. "omitted 90000 bytes" alone tells the
    /// model it lost something and gives it no way to get it back — which was
    /// the whole gap (#189 Phase 1).
    #[test]
    fn a_trimmed_result_names_its_spill_file_in_the_note() {
        let messages = spilled_history("/proj/.archon/spill/s/old-tool-Bash.txt");

        let projected = project_messages_for_request(&messages, 1);

        let note = projected[2]["content"][0]["content"]
            .as_str()
            .expect("projected content");
        assert!(note.contains("omitted"), "{note}");
        assert!(
            note.contains("/proj/.archon/spill/s/old-tool-Bash.txt"),
            "the note must name the file: {note}"
        );
        assert!(
            note.contains("read it if you need the omitted region"),
            "{note}"
        );
    }

    /// The key is archon's bookkeeping, recorded so it survives persistence and
    /// forking. It is not part of any provider's `tool_result` schema.
    #[test]
    fn the_spill_key_never_reaches_the_provider() {
        let messages = spilled_history("/proj/.archon/spill/s/old-tool-Bash.txt");

        let projected = project_messages_for_request(&messages, 1);

        let serialized = serde_json::to_string(&projected).expect("serialize");
        assert!(
            !serialized.contains(crate::agent::SPILL_PATH_KEY),
            "spill bookkeeping leaked onto the wire: {serialized}"
        );
    }

    /// Preserved recent turns skip trimming entirely, so they need their own
    /// stripping pass or the key rides out on them untouched.
    #[test]
    fn the_spill_key_is_stripped_from_preserved_recent_turns_too() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "only turn"}),
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "recent", "name": "Bash", "input": {}
            }]}),
            serde_json::json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "recent",
                "content": "small", "is_error": false,
                crate::agent::SPILL_PATH_KEY: "/proj/.archon/spill/s/recent-Bash.txt"
            }]}),
        ];

        let projected = project_messages_for_request(&messages, 5);

        let serialized = serde_json::to_string(&projected).expect("serialize");
        assert!(
            !serialized.contains(crate::agent::SPILL_PATH_KEY),
            "{serialized}"
        );
        assert_eq!(projected[2]["content"][0]["content"], "small");
        assert_eq!(projected[2]["content"][0]["tool_use_id"], "recent");
    }

    #[test]
    fn emergency_projection_also_strips_the_spill_key() {
        let messages = spilled_history("/proj/.archon/spill/s/old-tool-Bash.txt");

        let projected = project_messages_for_emergency_retry(&messages, 4_096);

        let serialized = serde_json::to_string(&projected).expect("serialize");
        assert!(
            !serialized.contains(crate::agent::SPILL_PATH_KEY),
            "{serialized}"
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
