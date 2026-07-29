#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextToolOutput {
    pub content: String,
    pub original_chars: usize,
    pub stored_chars: usize,
    pub limit_chars: usize,
    pub truncated: bool,
}

const DEFAULT_TOOL_RESULT_CONTEXT_CHARS: usize = 64_000;
const SHELL_TOOL_RESULT_CONTEXT_CHARS: usize = 24_000;
const SUBAGENT_TOOL_RESULT_CONTEXT_CHARS: usize = 32_000;

/// Hard ceiling applied when a tool result is FIRST recorded.
///
/// Deliberately not one of the constants above. Those are a *replay* budget —
/// how much of an old turn is worth re-sending once context has accumulated.
/// This is a different question: how large may a single block ever be. The two
/// happen to share a unit, and reusing one for the other would silently cost
/// every agent the recent-turn fidelity the preserve window exists to give it
/// (a `Bash` result would drop from `bash.rs`'s 102_400-byte cap to 24_000
/// chars the moment it was recorded).
///
/// Sized against the defect, not against the replay budget. The observed
/// failure was a single 18_031_035-char result against a provider per-field
/// limit of 10_485_760. 1_000_000 chars sits ~18x under the defect and ~10x
/// under the provider limit, while still being ~10x the largest legitimate
/// result the toolset can produce (`bash.rs` and `mcp_resources.rs` both stop
/// at 102_400 bytes) — so in practice nothing real is truncated here, and the
/// tool-aware limits above still do the actual context management on replay.
const INGEST_TOOL_RESULT_CEILING_CHARS: usize = 1_000_000;

/// Cap a tool result at the moment it is recorded.
///
/// Separate entry point from `cap_tool_output_for_context` so the ingest
/// ceiling and the replay budget cannot drift into each other by accident.
pub(crate) fn cap_tool_output_for_ingest(content: &str) -> ContextToolOutput {
    cap_to_limit(content, INGEST_TOOL_RESULT_CEILING_CHARS)
}

pub(crate) fn cap_tool_output_for_context(tool_name: &str, content: &str) -> ContextToolOutput {
    cap_to_limit(content, context_limit_for_tool(tool_name))
}

fn cap_to_limit(content: &str, limit_chars: usize) -> ContextToolOutput {
    let original_chars = content.chars().count();
    if original_chars <= limit_chars {
        return ContextToolOutput {
            content: content.to_string(),
            original_chars,
            stored_chars: original_chars,
            limit_chars,
            truncated: false,
        };
    }

    let marker = format!(
        "\n\n[Archon context note: tool output trimmed from {original_chars} chars before replaying it to the model. Full output was emitted to UI/logs.]\n\n"
    );
    let marker_chars = marker.chars().count();
    let body_budget = limit_chars.saturating_sub(marker_chars).max(1);
    let head_chars = body_budget / 2;
    let tail_chars = body_budget.saturating_sub(head_chars);

    let head: String = content.chars().take(head_chars).collect();
    let mut tail_vec: Vec<char> = content.chars().rev().take(tail_chars).collect();
    tail_vec.reverse();
    let tail: String = tail_vec.into_iter().collect();
    let trimmed = format!("{head}{marker}{tail}");
    let stored_chars = trimmed.chars().count();

    ContextToolOutput {
        content: trimmed,
        original_chars,
        stored_chars,
        limit_chars,
        truncated: true,
    }
}

pub(crate) fn project_messages_for_request(
    messages: &[serde_json::Value],
    preserve_recent_turns: u32,
) -> Vec<serde_json::Value> {
    let preserve_from = recent_turn_start(messages, preserve_recent_turns);
    let tool_names = tool_names_by_id(messages);
    let mut projected = messages.to_vec();

    for message in projected.iter_mut().take(preserve_from) {
        let Some(blocks) = message
            .get_mut("content")
            .and_then(|value| value.as_array_mut())
        else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|value| value.as_str()) != Some("tool_result") {
                continue;
            }
            let Some(content) = block.get("content").and_then(|value| value.as_str()) else {
                continue;
            };
            let tool_name = block
                .get("tool_use_id")
                .and_then(|value| value.as_str())
                .and_then(|id| tool_names.get(id))
                .map(String::as_str)
                .unwrap_or("");
            let output = cap_tool_output_for_context(tool_name, content);
            if output.truncated {
                block["content"] = serde_json::Value::String(output.content);
            }
        }
    }

    projected
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

fn context_limit_for_tool(tool_name: &str) -> usize {
    match tool_name {
        "Bash" | "Shell" => SHELL_TOOL_RESULT_CONTEXT_CHARS,
        "Agent" | "SendMessage" | "TaskCreate" | "TaskOutput" => SUBAGENT_TOOL_RESULT_CONTEXT_CHARS,
        _ => DEFAULT_TOOL_RESULT_CONTEXT_CHARS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tool_output_is_left_unchanged() {
        let output = cap_tool_output_for_context("Read", "small");

        assert!(!output.truncated);
        assert_eq!(output.content, "small");
        assert_eq!(output.original_chars, 5);
        assert_eq!(output.stored_chars, 5);
    }

    #[test]
    fn large_subagent_output_is_trimmed_for_context() {
        let content = format!("{}{}", "a".repeat(40_000), "z".repeat(40_000));
        let output = cap_tool_output_for_context("Agent", &content);

        assert!(output.truncated);
        assert_eq!(output.limit_chars, SUBAGENT_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.stored_chars <= SUBAGENT_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.content.contains("tool output trimmed"));
        assert!(output.content.starts_with('a'));
        assert!(output.content.ends_with('z'));
    }

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
        assert!(old.contains("tool output trimmed"));
        assert!(old.len() < messages[2]["content"][0]["content"].as_str().unwrap().len());
        assert_eq!(
            projected[5]["content"][0]["content"],
            messages[5]["content"][0]["content"]
        );
        assert_eq!(projected[5]["content"][0]["is_error"], true);
        assert_eq!(projected[5]["content"][0]["tool_use_id"], "recent-tool");
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

    #[test]
    fn giant_shell_output_gets_tighter_context_cap() {
        let content = format!("{}{}", "h".repeat(100_000), "t".repeat(100_000));
        let output = cap_tool_output_for_context("Bash", &content);

        assert!(output.truncated);
        assert_eq!(output.limit_chars, SHELL_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.stored_chars <= SHELL_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.content.contains("tool output trimmed"));
        assert!(output.content.starts_with('h'));
        assert!(output.content.ends_with('t'));
    }
}
