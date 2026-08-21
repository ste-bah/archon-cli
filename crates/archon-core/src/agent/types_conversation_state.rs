//! The live transcript an agent is working from.
//!
//! Split from `types.rs` to keep it under the file-size gate. Kept whole here
//! rather than divided further: the state and the operations that maintain its
//! invariants — tool-result pairing, spill accounting, compaction — only make
//! sense read together.

use super::*;

// ---------------------------------------------------------------------------
// Conversation state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConversationState {
    pub messages: Vec<serde_json::Value>,
    pub mode: AgentMode,
    pub max_tool_result_bytes: usize,
    /// Cumulative provider input tokens for billing/telemetry only.
    /// Auto-compaction triggers use last_known_context_tokens instead.
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Last API-reported full context size for this turn.
    /// Equals `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`
    /// from the Anthropic usage response — includes system prompt, tool schemas,
    /// memory injections, and messages. Used as the authoritative compaction trigger
    /// source. Falls back to `trigger_tokens(messages)` estimate when zero — on
    /// turn 1, after `/clear`, or transiently after a successful compaction (until
    /// the next API response repopulates it).
    pub last_known_context_tokens: u64,
    pub auto_compact: crate::agent::AutoCompactState,
    /// Where oversized tool results are written so their omitted region stays
    /// readable (#189 Phase 1). `None` disables spilling.
    pub spill: Option<SpillContext>,
    /// Correction applied to per-message token estimates (#189 Phase 3).
    ///
    /// Held here rather than on the surface itself because the surface is
    /// rebuilt whenever the message list changes, and this must outlive both
    /// that and the compaction that clears `last_known_context_tokens`.
    pub token_calibration: crate::agent::token_surface::Calibration,
}

/// What `add_tool_result` needs to write a spill file.
#[derive(Debug, Clone)]
pub struct SpillContext {
    pub working_dir: std::path::PathBuf,
    pub session_id: String,
    pub config: crate::config::SpillConfig,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            mode: AgentMode::Normal,
            max_tool_result_bytes: crate::agent::tool_result_context::DEFAULT_MAX_TOOL_RESULT_BYTES,
            total_input_tokens: 0,
            total_output_tokens: 0,
            last_known_context_tokens: 0,
            auto_compact: crate::agent::AutoCompactState::default(),
            spill: None,
            token_calibration: crate::agent::token_surface::Calibration::default(),
        }
    }
}

impl ConversationState {
    const INTERRUPTED_TOOL_RESULT: &'static str = "Tool dispatch interrupted before producing a result. \
         The assistant called this tool but no result was recorded, likely due \
         to mid-turn cancellation, dispatch panic, or session crash. Treat as failed.";

    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": content,
        }));
    }

    pub fn add_assistant_message(&mut self, content: Vec<serde_json::Value>) {
        self.messages.push(serde_json::json!({
            "role": "assistant",
            "content": content,
        }));
    }

    /// Record a tool result after applying the canonical ingest byte ceiling.
    pub fn add_tool_result(&mut self, tool_use_id: &str, content: &str, is_error: bool) {
        let capped = crate::agent::tool_result_context::cap_tool_output_to_bytes(
            content,
            self.max_tool_result_bytes,
        );
        if capped.truncated {
            tracing::warn!(
                tool_use_id,
                original_bytes = capped.original_bytes,
                stored_bytes = capped.stored_bytes,
                limit_bytes = capped.limit_bytes,
                "tool result exceeded the ingest ceiling and was truncated"
            );
        }
        let spilled = self.spill_locator_for(tool_use_id, content);
        let mut result = serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": capped.content,
            "is_error": is_error,
        });
        // Recorded on the block so it survives persistence and forking. The
        // request projection strips it before anything reaches the provider —
        // this key is archon's, not part of the tool_result wire format.
        if let Some(locator) = spilled {
            result[crate::agent::SPILL_PATH_KEY] =
                serde_json::Value::String(locator.path.display().to_string());
        }
        if let Some(last) = self.messages.last_mut()
            && last.get("role").and_then(|v| v.as_str()) == Some("user")
            && let Some(blocks) = last.get_mut("content").and_then(|v| v.as_array_mut())
            && !blocks.is_empty()
            && blocks
                .iter()
                .all(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
        {
            blocks.push(result);
            return;
        }
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [result],
        }));
    }

    pub(super) fn fill_missing_tool_results(&mut self, expected_ids: &[String]) -> Vec<String> {
        if expected_ids.is_empty() {
            return Vec::new();
        }
        let recorded_ids: std::collections::HashSet<String> = self
            .messages
            .last()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| {
                        block.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    })
                    .filter_map(|block| {
                        block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let missing: Vec<String> = expected_ids
            .iter()
            .filter(|id| !recorded_ids.contains(*id))
            .cloned()
            .collect();
        for id in &missing {
            self.add_tool_result(id, Self::INTERRUPTED_TOOL_RESULT, true);
        }
        missing
    }

    pub fn first_user_message(&self) -> &str {
        for msg in &self.messages {
            if msg["role"].as_str() == Some("user")
                && let Some(content) = msg["content"].as_str()
            {
                return content;
            }
        }
        ""
    }
}
