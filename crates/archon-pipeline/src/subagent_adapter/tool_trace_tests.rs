use super::*;

fn assistant(blocks: Vec<Value>) -> Value {
    json!({"role": "assistant", "content": blocks})
}

fn read_call(id: &str, path: &str) -> Value {
    json!({"type": "tool_use", "id": id, "name": "Read", "input": {"file_path": path}})
}

fn summary(uses: &[ToolUseEntry]) -> &Value {
    let last = uses.last().expect("a captured trace ends with its summary");
    assert_eq!(last.tool_name, TOOL_TRACE_SUMMARY_NAME);
    &last.input
}

#[test]
fn pairs_each_call_with_its_result_and_keeps_no_tool_output() {
    let messages = vec![
        json!({"role": "user", "content": "task"}),
        assistant(vec![
            json!({"type": "text", "text": "looking"}),
            read_call("t1", "a.rs"),
            json!({"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "false"}}),
        ]),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "SECRET BODY", "is_error": false},
            {"type": "tool_result", "tool_use_id": "t2", "content": "", "is_error": true},
        ]}),
        assistant(vec![
            json!({"type": "tool_use", "id": "t3", "name": "Grep", "input": {"pattern": "x"}}),
        ]),
    ];
    let uses = tool_uses(&messages);
    let names: Vec<_> = uses.iter().map(|u| u.tool_name.as_str()).collect();
    assert_eq!(names, ["Read", "Bash", "Grep", TOOL_TRACE_SUMMARY_NAME]);
    assert_eq!(uses[0].input["file_path"], "a.rs");
    assert_eq!(uses[0].output, json!({"is_error": false}));
    assert_eq!(uses[1].output, json!({"is_error": true}));
    assert_eq!(uses[2].output, Value::Null);
    let all = serde_json::to_string(
        &uses
            .iter()
            .map(|u| (&u.input, &u.output))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!all.contains("SECRET"), "{all}");
    assert_eq!(summary(&uses)["calls"], 3);
    assert_eq!(summary(&uses)["dropped"], 0);
}

#[test]
fn a_session_with_no_tool_call_still_yields_a_summary_of_zero_calls() {
    let messages = vec![
        json!({"role": "user", "content": "task"}),
        assistant(vec![json!({"type": "text", "text": "done"})]),
    ];
    let uses = tool_uses(&messages);
    assert_eq!(uses.len(), 1);
    assert_eq!(summary(&uses)["calls"], 0);
}

#[test]
fn history_without_an_assistant_message_was_not_captured_and_yields_nothing() {
    assert!(tool_uses(&[]).is_empty());
    assert!(tool_uses(&[json!({"role": "user", "content": "task"})]).is_empty());
}

/// A PEM private key block, built at run time so no literal key sits in the
/// source.
fn pem_block() -> String {
    let kind = "PRIVATE KEY";
    format!(
        "-----BEGIN {kind}-----\n{}\n-----END {kind}-----",
        "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC".repeat(40)
    )
}

#[test]
fn a_one_megabyte_write_keeps_its_path_and_no_content() {
    let huge = format!("{}{}", "x".repeat(1024 * 1024), pem_block());
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "w1", "name": "Write",
        "input": {"file_path": "big.txt", "content": huge, "nested": {"blob": huge}},
    })])];
    let uses = tool_uses(&messages);
    assert_eq!(uses[0].input, json!({"file_path": "big.txt"}));
    assert_eq!(uses[0].output["input_keys_dropped"], 2);
    let stored = serde_json::to_string(&uses).unwrap();
    assert!(stored.len() < 1024, "{} bytes stored", stored.len());
    assert!(!stored.contains("MIIEvQ"));
}

#[test]
fn a_write_of_service_account_json_stores_none_of_it() {
    let account = json!({"type": "service_account", "private_key_id": "abcdef0123456789",
        "private_key": pem_block()})
    .to_string();
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "w1", "name": "Write",
        "input": {"file_path": "sa.json", "content": account},
    })])];
    let stored = serde_json::to_string(&tool_uses(&messages)).unwrap();
    assert!(
        !stored.contains("abcdef0123456789") && !stored.contains("MIIEvQ"),
        "{stored}"
    );
}

#[test]
fn a_bash_command_is_kept_as_its_program_count_and_digest_only() {
    let command = format!(
        "cat <<'EOF' > key.pem\n{}\nEOF\nmysql -phunter2 {}",
        pem_block(),
        "x".repeat(900)
    );
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "b1", "name": "Bash", "input": {"command": command},
    })])];
    let uses = tool_uses(&messages);
    assert_eq!(uses[0].input["program"], "cat");
    assert!(uses[0].input.get("command").is_none());
    let stored = serde_json::to_string(&uses).unwrap();
    assert!(
        !stored.contains("MIIEvQ") && !stored.contains("hunter2"),
        "{stored}"
    );
    assert_eq!(summary(&uses)["inputs_truncated"], 0);
}

#[test]
fn a_non_object_input_is_dropped_and_counted() {
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "w1", "name": "Odd",
        "input": vec!["y".repeat(10_000); 50],
    })])];
    let uses = tool_uses(&messages);
    assert_eq!(uses[0].input, Value::Null);
    assert_eq!(uses[0].output["input_keys_dropped"], 1);
}

#[test]
fn many_calls_keep_the_first_bounded_number_and_count_the_rest() {
    let total = MAX_TRACE_CALLS + 57;
    let blocks = (0..total)
        .map(|i| read_call(&format!("r{i}"), &format!("f{i}.rs")))
        .collect();
    let uses = tool_uses(&[assistant(blocks)]);
    assert_eq!(uses.len(), MAX_TRACE_CALLS + 1);
    assert_eq!(
        uses[MAX_TRACE_CALLS - 1].input["file_path"],
        format!("f{}.rs", MAX_TRACE_CALLS - 1)
    );
    let summary = summary(&uses);
    assert_eq!(summary["calls"], total);
    assert_eq!(summary["kept"], MAX_TRACE_CALLS);
    assert_eq!(summary["dropped"], 57);
}
