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

#[test]
fn a_one_megabyte_input_is_cut_to_the_bound_and_the_cut_is_recorded() {
    let huge = "x".repeat(1024 * 1024);
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "w1", "name": "Write",
        "input": {"file_path": "big.txt", "content": huge, "nested": {"blob": huge}},
    })])];
    let uses = tool_uses(&messages);
    let kept = serde_json::to_string(&uses[0].input).unwrap();
    assert!(kept.len() <= MAX_INPUT_BYTES, "{} bytes kept", kept.len());
    assert_eq!(uses[0].input["file_path"], "big.txt");
    assert!(
        uses[0].input["content"]
            .as_str()
            .unwrap()
            .ends_with('\u{2026}')
    );
    assert_eq!(uses[0].output["input_truncated"], true);
    assert!(uses[0].output["input_bytes"].as_u64().unwrap() > 2 * 1024 * 1024);
    assert_eq!(summary(&uses)["inputs_truncated"], 1);
}

#[test]
fn a_huge_non_object_input_is_cut_too() {
    let messages = vec![assistant(vec![json!({
        "type": "tool_use", "id": "w1", "name": "Odd",
        "input": vec!["y".repeat(10_000); 50],
    })])];
    let uses = tool_uses(&messages);
    assert!(serde_json::to_string(&uses[0].input).unwrap().len() <= MAX_INPUT_BYTES);
    assert_eq!(uses[0].output["input_truncated"], true);
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

#[test]
fn clip_cuts_on_a_character_boundary() {
    let text = "\u{e9}".repeat(100);
    let cut = clip(&text, 11);
    assert!(cut.len() <= 11);
    assert!(cut.ends_with('\u{2026}'));
}
