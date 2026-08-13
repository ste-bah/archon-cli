//! Keep the volatile part of a turn behind the cacheable prefix.
//!
//! Every provider caches by common prefix. Archon builds its system prompt as
//! the configured blocks followed by whatever this turn produced — recalled
//! memories, the inner voice, the guardrail and cognitive reminders — which puts
//! content that changes almost every turn *in front of* the tools and the entire
//! message history. The prefix is only a hit up to its first changed byte, so
//! one different memory invalidates everything behind it and the whole
//! conversation is re-read at full price.
//!
//! Where a breakpoint exists that can be patched over (see
//! `request_cache::apply_stable_system_cache`). Where one does not — GPT-5.5 and
//! earlier, DeepSeek, and every other provider that caches implicitly — the
//! ordering is the only lever there is.
//!
//! So the volatile blocks move to the end of the last user message, which is
//! where archon's `<system-reminder>` content already goes and which changes
//! every turn regardless. The stable system prompt, the tools and the full
//! history become one uninterrupted prefix.

/// Move the per-turn system blocks onto the last user message.
///
/// `stable_blocks` is how many leading entries of `system` came from
/// configuration. Everything after them was appended by this turn.
///
/// Returns the number of blocks moved. Zero means the request was left exactly
/// as it was: there was nothing volatile, or there was no user message to carry
/// it, in which case leaving it in the system prompt is still correct — it is
/// only the position that is being improved, never the content.
pub(super) fn move_volatile_system_to_last_turn(
    system: &mut Vec<serde_json::Value>,
    messages: &mut [serde_json::Value],
    stable_blocks: usize,
) -> usize {
    if stable_blocks == 0 || stable_blocks >= system.len() {
        return 0;
    }

    let Some(target) = messages
        .iter_mut()
        .rev()
        .find(|message| message.get("role").and_then(|r| r.as_str()) == Some("user"))
    else {
        // No user turn to carry them. This happens on the subagent path, where
        // the messages are projected later — and there the system prompt is
        // stable across rounds anyway, so there is nothing to gain.
        return 0;
    };

    let volatile = system.split_off(stable_blocks);
    let moved = volatile.len();

    // `content` is a bare string on a plain user turn and an array once tool
    // results are in play. Normalise before appending, rather than growing a
    // second shape for callers downstream to handle.
    let content = target
        .as_object_mut()
        .and_then(|object| object.get_mut("content"));
    let Some(content) = content else {
        // Malformed message: put the blocks back rather than dropping them.
        system.extend(volatile);
        return 0;
    };

    if let Some(text) = content.as_str() {
        *content = serde_json::json!([{ "type": "text", "text": text }]);
    }
    let Some(blocks) = content.as_array_mut() else {
        system.extend(volatile);
        return 0;
    };

    for block in volatile {
        let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
        if text.is_empty() {
            continue;
        }
        blocks.push(serde_json::json!({ "type": "text", "text": text }));
    }

    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> serde_json::Value {
        serde_json::json!({ "type": "text", "text": text })
    }

    fn user(text: &str) -> serde_json::Value {
        serde_json::json!({ "role": "user", "content": text })
    }

    #[test]
    fn the_volatile_tail_moves_onto_the_last_user_message() {
        let mut system = vec![block("stable"), block("recalled memory")];
        let mut messages = vec![
            user("first"),
            serde_json::json!({"role":"assistant","content":"reply"}),
            user("latest"),
        ];

        let moved = move_volatile_system_to_last_turn(&mut system, &mut messages, 1);

        assert_eq!(moved, 1);
        assert_eq!(system.len(), 1, "only the configured blocks remain");
        assert_eq!(messages[2]["content"][0]["text"], "latest");
        assert_eq!(
            messages[2]["content"][1]["text"], "recalled memory",
            "the volatile block lands after the user's own text"
        );
        assert_eq!(
            messages[0]["content"], "first",
            "earlier turns are untouched, which is the whole point"
        );
    }

    #[test]
    fn an_existing_content_array_is_appended_to_rather_than_replaced() {
        let mut system = vec![block("stable"), block("reminder")];
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]
        })];

        move_volatile_system_to_last_turn(&mut system, &mut messages, 1);

        let blocks = messages[0]["content"].as_array().expect("array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[1]["text"], "reminder");
    }

    #[test]
    fn nothing_volatile_means_nothing_moves() {
        let mut system = vec![block("stable")];
        let mut messages = vec![user("hi")];

        assert_eq!(
            move_volatile_system_to_last_turn(&mut system, &mut messages, 1),
            0
        );
        assert_eq!(system.len(), 1);
        assert_eq!(messages[0]["content"], "hi");
    }

    /// Content is never dropped to improve a cache hit.
    #[test]
    fn with_no_user_message_the_blocks_stay_in_the_system_prompt() {
        let mut system = vec![block("stable"), block("reminder")];
        let mut messages = vec![serde_json::json!({"role":"assistant","content":"only"})];

        assert_eq!(
            move_volatile_system_to_last_turn(&mut system, &mut messages, 1),
            0
        );
        assert_eq!(system.len(), 2);
    }

    #[test]
    fn a_malformed_target_message_gives_the_blocks_back() {
        let mut system = vec![block("stable"), block("reminder")];
        let mut messages = vec![serde_json::json!({"role":"user"})];

        assert_eq!(
            move_volatile_system_to_last_turn(&mut system, &mut messages, 1),
            0
        );
        assert_eq!(system.len(), 2, "the reminder must not be lost");
    }
}
