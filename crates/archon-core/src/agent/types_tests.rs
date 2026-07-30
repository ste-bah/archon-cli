use super::*;

#[test]
fn runtime_context_extra_carries_agent_identity() {
    let config = AgentConfig {
        session_id: "session-1".to_string(),
        agent_type: "reviewer".to_string(),
        agent_version: Some("1.0.0".to_string()),
        ..AgentConfig::default()
    };

    let extra =
        config.runtime_attribution_extra("assistant", "main_session", Some(2), Some(3), Some(100));

    assert_eq!(extra["archon_runtime"]["run_id"], "session-1");
    assert_eq!(extra["archon_runtime"]["session_id"], "session-1");
    assert_eq!(extra["archon_runtime"]["role"], "assistant");
    assert_eq!(extra["archon_runtime"]["agent_type"], "reviewer");
    assert_eq!(extra["archon_runtime"]["agent_version"], "1.0.0");
    assert_eq!(extra["archon_runtime"]["turn"], 2);
    assert_eq!(extra["archon_runtime"]["round"], 3);
    assert_eq!(extra["archon_runtime"]["effective_denominator"], 100);
}

#[test]
fn conversation_state_batches_adjacent_tool_results() {
    let mut state = ConversationState::default();
    state.add_assistant_message(vec![serde_json::json!({
        "type": "tool_use",
        "id": "tool-1",
        "name": "Read",
        "input": {}
    })]);

    state.add_tool_result("tool-1", "one", false);
    state.add_tool_result("tool-2", "two", false);

    assert_eq!(state.messages.len(), 2);
    let blocks = state.messages[1]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["tool_use_id"], "tool-1");
    assert_eq!(blocks[1]["tool_use_id"], "tool-2");
}

#[test]
fn conversation_state_caps_tool_result_text_at_ingest() {
    let mut state = ConversationState::default();
    let huge = "x".repeat(2_000_000);

    state.add_tool_result("tool-1", &huge, false);

    let content = state.messages[0]["content"][0]["content"]
        .as_str()
        .expect("tool result content");
    assert!(content.chars().count() < huge.chars().count());
    assert!(content.contains("tool output trimmed"));
}

#[test]
fn realistically_large_tool_results_survive_ingest_untouched() {
    let mut state = ConversationState::default();
    let biggest_real_result = "x".repeat(102_400);

    state.add_tool_result("tool-1", &biggest_real_result, false);

    assert_eq!(
        state.messages[0]["content"][0]["content"]
            .as_str()
            .expect("tool result content"),
        biggest_real_result
    );
}

#[test]
fn no_stored_tool_result_can_exceed_the_provider_per_field_limit() {
    const PROVIDER_PER_FIELD_LIMIT: usize = 10_485_760;
    let mut state = ConversationState::default();
    state.add_assistant_message(vec![
        serde_json::json!({"type": "tool_use", "id": "grep-1", "name": "Grep", "input": {}}),
    ]);

    state.add_tool_result("grep-1", &"x".repeat(18_031_035), false);

    let stored = state.messages[1]["content"][0]["content"]
        .as_str()
        .unwrap()
        .chars()
        .count();
    assert!(
        stored < PROVIDER_PER_FIELD_LIMIT,
        "stored {stored} chars still exceeds the provider's per-field limit"
    );
}

#[test]
fn small_tool_results_are_stored_verbatim() {
    let mut state = ConversationState::default();
    state.add_assistant_message(vec![
        serde_json::json!({"type": "tool_use", "id": "tool-1", "name": "Bash", "input": {}}),
    ]);

    state.add_tool_result("tool-1", "exit 0", false);

    assert_eq!(state.messages[1]["content"][0]["content"], "exit 0");
}
