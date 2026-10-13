use super::*;

fn prepared_tool() -> PreparedTool {
    PreparedTool {
        id: "tool-1".into(),
        name: "Read".into(),
        input: Ok(serde_json::json!({})),
        raw_input: "{}".into(),
    }
}

#[test]
fn raw_transcript_keeps_full_result_while_canonical_message_is_byte_bounded() {
    let raw_content = format!("HEAD{}TAIL", "é".repeat(100_000));
    let (raw_transcript, canonical) = build_tool_result_messages(
        &[prepared_tool()],
        vec![ToolResult::success(raw_content.clone())],
        4_096,
    );

    assert_eq!(
        raw_transcript["content"][0]["content"],
        serde_json::Value::String(raw_content)
    );
    let canonical_content = canonical["content"][0]["content"]
        .as_str()
        .expect("canonical tool result content");
    assert!(canonical_content.contains("omitted"));
    assert!(
        serde_json::to_vec(&serde_json::Value::String(canonical_content.into()))
            .expect("serialize canonical provider field")
            .len()
            <= 4_096
    );
}

#[test]
fn raw_transcript_reopens_with_full_result_while_canonical_stays_capped() {
    let raw_content = format!("HEAD{}TAIL", "é".repeat(100_000));
    let (raw_transcript, canonical) = build_tool_result_messages(
        &[prepared_tool()],
        vec![ToolResult::success(raw_content.clone())],
        4_096,
    );
    let temp = tempfile::tempdir().expect("create transcript directory");
    let store =
        crate::agents::transcript::AgentTranscriptStore::with_base_dir(temp.path().to_path_buf());

    store.record_message("agent-1", &raw_transcript);
    let reopened =
        crate::agents::transcript::AgentTranscriptStore::with_base_dir(temp.path().to_path_buf());
    let persisted = reopened
        .get_transcript("agent-1")
        .expect("reload raw transcript");

    assert_eq!(persisted[0]["content"][0]["content"], raw_content);
    assert_ne!(canonical["content"][0]["content"], raw_content);
    assert!(
        canonical["content"][0]["content"]
            .as_str()
            .expect("canonical content")
            .contains("omitted")
    );
}

/// #171 part 8: one parse per tool must not change what a tool whose
/// input never parsed replays, or what it hands the executor.
#[test]
fn malformed_tool_input_replays_the_marker_and_fails_execution_with_the_same_error() {
    let malformed = PreparedTool {
        id: "tool-9".into(),
        name: "Read".into(),
        input: Err("tool input was not valid JSON".into()),
        raw_input: "{not json".into(),
    };

    assert_eq!(
        replay_tool_input(&malformed),
        serde_json::json!({
            "_archon_malformed_tool_input": true,
            "error": "tool input was not valid JSON",
        })
    );
    assert_eq!(
        malformed.input.unwrap_err(),
        "tool input was not valid JSON",
        "execution surfaces the parse error verbatim"
    );
}

#[test]
fn ordinary_tool_result_is_identical_in_raw_and_canonical_messages() {
    let (raw_transcript, canonical) = build_tool_result_messages(
        &[prepared_tool()],
        vec![ToolResult::success("small output")],
        4_096,
    );

    assert_eq!(raw_transcript, canonical);
}
