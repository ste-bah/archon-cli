/// The subagent's live message history together with its running token
/// estimate (#171 part 1).
///
/// `estimate_messages_tokens` serializes every message to measure it, and a
/// round asked for that number twice — once for the context-window trigger and
/// once for the request-pressure check — so a 400KB transcript was serialized
/// roughly 800KB worth per round purely to produce two integers.
///
/// The estimate is a plain sum over messages, so it is maintained
/// incrementally instead. Every mutation goes through this type, which is what
/// keeps the running value honest: `push` adds exactly the term
/// `estimate_messages_tokens` would have added, and `replace` — the only way
/// history is rewritten, by compaction or a recovery projection — recomputes
/// from scratch. `message_history_tests` pins that equivalence.
pub(super) struct MessageHistory {
    messages: Vec<serde_json::Value>,
    estimated_tokens: u64,
}

impl MessageHistory {
    pub(super) fn new(messages: Vec<serde_json::Value>) -> Self {
        let estimated_tokens = crate::agent::autocompact::estimate_messages_tokens(&messages);
        Self {
            messages,
            estimated_tokens,
        }
    }

    /// Append a message, folding its estimate into the running total.
    pub(super) fn push(&mut self, message: serde_json::Value) {
        self.estimated_tokens = self
            .estimated_tokens
            .saturating_add(crate::agent::autocompact::estimate_message_tokens(&message));
        self.messages.push(message);
    }

    /// Rewrite the whole history and recompute the estimate once.
    ///
    /// Compaction and the emergency projection do not preserve any per-message
    /// relationship with the old array, so there is nothing to fold — this is
    /// the single O(N) pass the issue's acceptance criteria allow.
    pub(super) fn replace(&mut self, messages: Vec<serde_json::Value>) {
        self.estimated_tokens = crate::agent::autocompact::estimate_messages_tokens(&messages);
        self.messages = messages;
    }

    pub(super) fn as_slice(&self) -> &[serde_json::Value] {
        &self.messages
    }

    /// Running estimate, equal to `estimate_messages_tokens(self.as_slice())`.
    pub(super) fn estimated_tokens(&self) -> u64 {
        self.estimated_tokens
    }
}

#[cfg(test)]
mod message_history_tests {
    use super::MessageHistory;

    fn tool_result(index: usize, bytes: usize) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": format!("tool-{index}"),
                "content": "x".repeat(bytes),
                "is_error": false,
            }],
        })
    }

    fn assert_matches_full_estimate(history: &MessageHistory) {
        assert_eq!(
            history.estimated_tokens(),
            crate::agent::autocompact::estimate_messages_tokens(history.as_slice()),
            "the running estimate must equal a full recount"
        );
    }

    #[test]
    fn running_estimate_tracks_a_full_recount_across_pushes() {
        let mut history = MessageHistory::new(vec![serde_json::json!({
            "role": "user",
            "content": "start",
        })]);
        assert_matches_full_estimate(&history);

        for index in 0..8 {
            history.push(tool_result(index, 4_096 * (index + 1)));
            assert_matches_full_estimate(&history);
        }
    }

    #[test]
    fn replace_recomputes_after_compaction_style_rewrites() {
        let mut history = MessageHistory::new((0..6).map(|i| tool_result(i, 8_192)).collect());
        let before = history.estimated_tokens();

        history.replace(vec![serde_json::json!({
            "role": "user",
            "content": "Context Summary: older conversation messages were compacted.",
        })]);

        assert!(history.estimated_tokens() < before);
        assert_matches_full_estimate(&history);

        history.push(tool_result(99, 16_384));
        assert_matches_full_estimate(&history);
    }

    #[test]
    fn empty_history_estimates_zero() {
        let mut history = MessageHistory::new(Vec::new());

        assert_eq!(history.estimated_tokens(), 0);
        assert_matches_full_estimate(&history);

        history.replace(Vec::new());
        assert_eq!(history.estimated_tokens(), 0);
    }
}
