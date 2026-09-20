//! What the compaction summariser is allowed to see, and how much it may write.
//!
//! Both were live failures on 2026-09-08: the summariser was handed raw tool
//! output and given 2048 tokens to compress it, so the summary was cut off
//! mid-sentence, the conversation never shrank, and the run overflowed the
//! context window six times without reaching implementation.
use super::summary_text::to_summary_context_messages;
use super::types::AgentConfig;
use serde_json::json;

fn text_of(messages: &[serde_json::Value]) -> String {
    to_summary_context_messages(messages)
        .into_iter()
        .map(|message| message.content.as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn raw_tool_output_never_reaches_the_summariser() {
    let build_log = "error[E0432]: unresolved import\n".repeat(500);
    let messages = vec![
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "Running the build to check the wiring."},
            {"type": "tool_use", "id": "t1", "name": "Bash",
             "input": {"command": "cargo build --release --bin archon"}}
        ]}),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": build_log}
        ]}),
    ];
    let rendered = text_of(&messages);

    // The assistant's own words are what carry intent forward, so they stay.
    assert!(rendered.contains("Running the build to check the wiring."));
    // The tool is named, so the summary can say what happened.
    assert!(rendered.contains("[Called tool: Bash]"), "got: {rendered}");
    // Neither the arguments nor the output are reproduced.
    assert!(
        !rendered.contains("cargo build --release"),
        "arguments leaked"
    );
    assert!(!rendered.contains("E0432"), "tool output leaked");
    assert!(
        rendered.len() < 200,
        "a 15 KB build log still reached the summariser: {} bytes",
        rendered.len()
    );
}

#[test]
fn the_summary_budget_follows_configured_max_tokens() {
    let budget = |max_tokens: u32| {
        let mut config = AgentConfig::default();
        config.max_tokens = max_tokens;
        config.compaction_summary_max_tokens()
    };
    // Half the answer ceiling, so it moves with config.toml...
    assert_eq!(budget(32_768), 16_384);
    assert_eq!(budget(16_384), 8_192);
    assert_eq!(budget(8_192), 4_096);
    // ...floored, so a small ceiling cannot make summaries useless...
    assert_eq!(budget(2_048), 2_048);
    assert_eq!(budget(1), 2_048);
    // ...and capped, because a summary that large has stopped compacting.
    assert_eq!(budget(131_072), 16_384);
    // Never above what one response is allowed to produce.
    for max_tokens in [2_048u32, 8_192, 16_384, 32_768, 65_536, 131_072] {
        assert!(
            budget(max_tokens) <= max_tokens.max(2_048),
            "summary budget exceeds the answer ceiling at {max_tokens}"
        );
    }
}

#[test]
fn a_truncated_summary_is_a_structural_failure_not_a_success() {
    use crate::agent::autocompact::CompactionError;
    use crate::agent::autocompact::{CompactionFailureDisposition, compaction_failure_disposition};
    // The classification is what makes suppression reachable: structural
    // failures accumulate toward `disabled`, whereas the old path returned Ok
    // and `on_success` reset the counters on every doomed attempt.
    let error = CompactionError::InvalidSummary("summary was truncated".into());
    assert_eq!(
        compaction_failure_disposition(&error),
        CompactionFailureDisposition::Structural
    );
}

#[test]
fn repeated_structural_failures_eventually_stop_compaction_retrying() {
    use crate::agent::AutoCompactState;
    use crate::agent::autocompact::CompactionError;
    let mut state = AutoCompactState::default();
    assert!(state.should_attempt());
    for _ in 0..16 {
        state.on_failure(&CompactionError::InvalidSummary("truncated".into()));
    }
    assert!(
        !state.should_attempt(),
        "compaction still retrying after repeated structural failures"
    );
}

#[test]
fn a_compaction_that_reclaims_nothing_is_a_failure_not_a_success() {
    use crate::agent::autocompact::CompactionError;
    use crate::agent::autocompact::{CompactionFailureDisposition, compaction_failure_disposition};
    // Outcome-based, not text-based: the summary can be complete and well
    // formed while the conversation ends up no smaller. That was reported as
    // success, resetting the failure counters, so compaction re-ran every turn
    // and reclaimed nothing each time. It must accumulate toward suppression.
    let error = CompactionError::InvalidSummary(
        "compaction reclaimed nothing: 100 tokens before, 120 after".into(),
    );
    assert_eq!(
        compaction_failure_disposition(&error),
        CompactionFailureDisposition::Structural
    );
}
