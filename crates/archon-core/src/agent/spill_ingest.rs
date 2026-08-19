//! Deciding what to spill, at the moment a tool result is recorded (#189).
//!
//! Spilling happens once per tool call, here, rather than where the trimming
//! happens. The request projection trims a fresh copy on every request, so
//! spilling there would rewrite the same file each turn; and compaction trims
//! through the same pure helper, where a file per trimmed message is noise.
//!
//! Split from `types.rs` to keep that file under the 500-line ceiling.

use super::types::ConversationState;
use crate::spill::SpillLocator;

/// Key under which a tool_result block records where its full output went.
///
/// Archon's own, not part of any provider's tool_result schema — the request
/// projection removes it before the message reaches the wire.
pub const SPILL_PATH_KEY: &str = "archon_spill";

/// Build a session's spill context, sweeping expired directories on the way.
///
/// Pruning happens at session start rather than on a timer: a spill directory
/// nobody has opened archon to look at is not urgent, and a background sweeper
/// is one more thing that can delete the wrong path.
pub(super) fn open_spill(
    working_dir: &std::path::Path,
    session_id: &str,
) -> Option<super::types::SpillContext> {
    let config = crate::config::load_config()
        .map(|loaded| loaded.spill)
        .unwrap_or_default();
    let pruned = crate::spill::prune(working_dir, config.retention());
    if pruned > 0 {
        tracing::debug!(pruned, "pruned expired tool-output spill directories");
    }
    Some(super::types::SpillContext {
        working_dir: working_dir.to_path_buf(),
        session_id: session_id.to_string(),
        config,
    })
}

impl ConversationState {
    /// Write this result's full output if losing part of it would cost
    /// something, and return where it went.
    pub(super) fn spill_locator_for(
        &self,
        tool_use_id: &str,
        content: &str,
    ) -> Option<SpillLocator> {
        let context = self.spill.as_ref()?;
        if !context.config.enabled {
            return None;
        }
        let tool_name = self.tool_name_for(tool_use_id)?;
        if !crate::spill::is_spillable(&tool_name) {
            return None;
        }
        if !super::tool_result_context::exceeds_context_budget(&tool_name, content) {
            return None;
        }
        match crate::spill::save(
            &context.working_dir,
            &context.session_id,
            &tool_name,
            tool_use_id,
            content,
        ) {
            Ok(locator) => Some(locator),
            Err(error) => {
                // A full disk costs the retrieval path, not the turn. The note
                // simply goes out without a filename.
                tracing::warn!(%error, tool_use_id, "could not spill tool output");
                None
            }
        }
    }

    /// Find which tool produced this result.
    ///
    /// The per-tool replay budget is what decides whether anything will be
    /// omitted, and that budget is keyed by name — but `add_tool_result` is
    /// given only the id. The matching `tool_use` block is already in history
    /// by then, so the name is recoverable without threading it through the
    /// fifteen call sites that record results.
    ///
    /// Searched newest-first: the block was emitted moments ago, and ids are
    /// unique, so this stops almost immediately.
    fn tool_name_for(&self, tool_use_id: &str) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .filter_map(|message| message.get("content").and_then(|value| value.as_array()))
            .flatten()
            .find(|block| {
                block.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                    && block.get("id").and_then(|v| v.as_str()) == Some(tool_use_id)
            })
            .and_then(|block| block.get("name").and_then(|v| v.as_str()))
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::SpillContext;
    use crate::config::SpillConfig;

    fn state_with_tool(dir: &std::path::Path, tool: &str) -> ConversationState {
        let mut state = ConversationState {
            spill: Some(SpillContext {
                working_dir: dir.to_path_buf(),
                session_id: "sess-1".to_string(),
                config: SpillConfig::default(),
            }),
            ..ConversationState::default()
        };
        state.add_assistant_message(vec![serde_json::json!({
            "type": "tool_use", "id": "call-1", "name": tool, "input": {}
        })]);
        state
    }

    /// The shell budget is 24 KB, well under the 1 MB ingest ceiling. Keying
    /// spill to the ingest ceiling instead would leave this result — trimmed on
    /// every single request — with no file at all.
    #[test]
    fn a_shell_result_over_its_replay_budget_is_spilled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = state_with_tool(dir.path(), "Bash");
        let content = "x".repeat(30_000);

        let locator = state
            .spill_locator_for("call-1", &content)
            .expect("30 KB of shell output is over the 24 KB replay budget");

        assert_eq!(locator.bytes, content.len());
        assert_eq!(
            std::fs::read_to_string(&locator.path).expect("read back"),
            content
        );
    }

    #[test]
    fn a_result_within_its_budget_is_not_spilled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = state_with_tool(dir.path(), "Bash");

        assert!(state.spill_locator_for("call-1", "small output").is_none());
        assert!(
            !crate::spill::spill_root(dir.path()).exists(),
            "nothing to retrieve means no directory to prune later"
        );
    }

    /// Re-reading the path beats reading a copy that may already be stale.
    #[test]
    fn a_file_backed_read_is_not_copied_to_a_second_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = state_with_tool(dir.path(), "Read");

        assert!(
            state
                .spill_locator_for("call-1", &"x".repeat(200_000))
                .is_none(),
            "Read results are already retrievable from their own path"
        );
    }

    #[test]
    fn nothing_is_written_when_spilling_is_disabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_tool(dir.path(), "Bash");
        if let Some(context) = state.spill.as_mut() {
            context.config = SpillConfig {
                enabled: false,
                ..SpillConfig::default()
            };
        }

        assert!(
            state
                .spill_locator_for("call-1", &"x".repeat(200_000))
                .is_none()
        );
    }

    /// Subagent and compaction paths build states with no spill context. They
    /// must record results as before rather than panic or write somewhere.
    #[test]
    fn a_state_without_a_spill_context_records_normally() {
        let state = ConversationState::default();
        assert!(
            state
                .spill_locator_for("call-1", &"x".repeat(200_000))
                .is_none()
        );
    }

    /// Without a matching `tool_use` block there is no name, so no budget to
    /// judge against — guessing a budget would spill the wrong things.
    #[test]
    fn an_unmatched_id_is_not_spilled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = state_with_tool(dir.path(), "Bash");

        assert!(
            state
                .spill_locator_for("other-call", &"x".repeat(200_000))
                .is_none()
        );
    }

    /// The acceptance criterion, end to end: a shell result past its replay
    /// budget is written to disk, the note the model receives names that file,
    /// and reading it returns the original bytes exactly.
    #[test]
    fn an_oversized_shell_result_is_recorded_spilled_and_retrievable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_tool(dir.path(), "Bash");
        let content = format!("HEAD{}TAIL", "y".repeat(40_000));

        state.add_tool_result("call-1", &content, false);
        // A later turn, so the result is outside the preserved window and is
        // actually trimmed on its way to the provider.
        state.add_user_message("next turn");

        let projected =
            super::super::tool_result_context::project_messages_for_request(&state.messages, 1);
        let note = projected
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_array()))
            .flatten()
            .find(|b| b.get("tool_use_id").and_then(|v| v.as_str()) == Some("call-1"))
            .and_then(|b| b.get("content").and_then(|v| v.as_str()))
            .expect("the tool result survives projection");

        assert!(note.contains("omitted"), "{note}");
        let path = note
            .split("Full output: ")
            .nth(1)
            .and_then(|rest| rest.split(" (read it").next())
            .expect("the note names a path: {note}");
        assert_eq!(
            std::fs::read_to_string(path).expect("read the spilled output"),
            content,
            "the spilled file must hold the complete original, byte for byte"
        );
    }

    /// Compaction trims through `cap_tool_output_to_bytes` — `autocompact.rs`
    /// and `segment_compaction.rs` both call it — and so does the request
    /// projection, on every single request. It takes no spill context and must
    /// never gain one: a file per trimmed message would be noise, and the
    /// projection would rewrite the same file every turn.
    #[test]
    fn the_shared_trimming_helper_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");

        let trimmed = crate::agent::tool_result_context::cap_tool_output_to_bytes(
            &"x".repeat(200_000),
            1_000,
        );

        assert!(trimmed.truncated, "the fixture must actually be trimmed");
        assert!(
            !crate::spill::spill_root(dir.path()).exists(),
            "compaction-internal trimming must not produce spill files"
        );
    }

    #[test]
    fn the_tool_name_is_recovered_from_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = state_with_tool(dir.path(), "Grep");

        assert_eq!(state.tool_name_for("call-1").as_deref(), Some("Grep"));
        assert_eq!(state.tool_name_for("missing"), None);
    }
}
