use super::*;

fn call(name: &str, input: Value, output: Value) -> WorkflowAgentToolUse {
    WorkflowAgentToolUse {
        tool_name: name.to_string(),
        input,
        output,
    }
}

fn ok() -> Value {
    json!({"is_error": false})
}

/// The summary the host session trace ends with.
fn summary(calls: u64, kept: u64, cut: u64) -> WorkflowAgentToolUse {
    call(
        TOOL_TRACE_SUMMARY_NAME,
        json!({"calls": calls, "kept": kept, "dropped": calls - kept, "inputs_truncated": cut}),
        Value::Null,
    )
}

/// Token-shaped values, built at run time so no literal credential sits in
/// the source.
fn github_token() -> String {
    format!("ghp_{}", "A1b2C3d4E5".repeat(4).get(..36).unwrap())
}

fn anthropic_key() -> String {
    format!("sk-ant-api03-{}", "Zz9".repeat(10))
}

#[test]
fn reads_become_files_read_once_and_other_calls_become_commands() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call("Read", json!({"file_path": "a.rs"}), ok()),
            call("Read", json!({"file_path": "a.rs"}), ok()),
            call("Glob", json!({"pattern": "**/*.rs"}), ok()),
            call("Bash", json!({"command": "ls"}), Value::Null),
            summary(4, 4, 0),
        ],
    );
    assert_eq!(result.files_read.len(), 1);
    assert_eq!(result.files_read[0].path, "a.rs");
    let commands: Vec<_> = result
        .commands_run
        .iter()
        .map(|c| (c.command.as_str(), c.status))
        .collect();
    assert_eq!(commands.len(), 2, "{commands:?}");
    assert!(commands[0].0.starts_with("Glob ") && commands[0].0.contains("**/*.rs"));
    assert!(commands[1].0.starts_with("ls (0 args)"), "{commands:?}");
    assert_eq!(commands[1].1, WorkflowV2CommandStatus::Skipped);
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], true);
    assert_eq!(trace["source"], "host session trace");
    assert_eq!(trace["toolCalls"], 4);
    assert_eq!(trace["complete"], true);
}

#[test]
fn a_failed_read_is_a_failed_command_not_a_file_read() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call(
                "Read",
                json!({"file_path": "gone.rs"}),
                json!({"is_error": true}),
            ),
            summary(1, 1, 0),
        ],
    );
    assert!(result.files_read.is_empty(), "{:?}", result.files_read);
    assert_eq!(result.commands_run.len(), 1);
    assert!(result.commands_run[0].command.contains("gone.rs"));
    assert_eq!(
        result.commands_run[0].status,
        WorkflowV2CommandStatus::Failed
    );
}

#[test]
fn a_captured_trace_of_zero_calls_stores_empty_lists_as_fact() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(&mut result, &[summary(0, 0, 0)]);
    assert!(result.files_read.is_empty());
    assert!(result.commands_run.is_empty());
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], true, "{trace}");
    assert_eq!(trace["toolCalls"], 0);
    assert!(!trace.to_string().contains(NOT_RECORDED), "{trace}");
    assert!(
        result
            .evidence
            .iter()
            .all(|e| !e.summary.contains("not recorded"))
    );
}

#[test]
fn no_trace_is_marked_not_recorded_and_keeps_existing_data() {
    let mut result = WorkflowV2Result::accepted("raw");
    result.data = json!({"content": "x"});
    record_tool_trace(&mut result, &[]);
    assert_eq!(result.data["content"], "x");
    assert_eq!(result.data["toolTrace"]["recorded"], false);
    assert_eq!(result.data["toolTrace"]["filesRead"], NOT_RECORDED);
    assert_eq!(result.data["toolTrace"]["commandsRun"], NOT_RECORDED);
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("not recorded"))
    );
}

#[test]
fn calls_without_a_host_summary_are_recorded_but_not_claimed_complete() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[call("Read", json!({"file_path": "a.rs"}), ok())],
    );
    assert_eq!(result.files_read.len(), 1);
    assert_eq!(result.data["toolTrace"]["recorded"], true);
    assert_eq!(result.data["toolTrace"]["complete"], false);
}

#[test]
fn dropped_calls_are_counted_and_said_in_evidence() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call("Read", json!({"file_path": "a.rs"}), ok()),
            summary(9, 1, 0),
        ],
    );
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["toolCalls"], 9);
    assert_eq!(trace["kept"], 1);
    assert_eq!(trace["dropped"], 8);
    assert_eq!(trace["complete"], false);
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("kept the first 1"))
    );
}

#[test]
fn a_read_whose_input_the_trace_cut_is_not_claimed_as_a_file_read() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call(
                "Read",
                json!({"file_path": "very/long\u{2026}"}),
                json!({"is_error": false, "input_truncated": true, "input_bytes": 9000}),
            ),
            summary(1, 1, 1),
        ],
    );
    assert!(result.files_read.is_empty());
    assert!(result.commands_run[0].output_summary.contains("input cut"));
    assert_eq!(result.data["toolTrace"]["inputsTruncated"], 1);
}

#[test]
fn a_credential_in_a_grep_pattern_or_a_bash_command_is_redacted() {
    let (github, anthropic) = (github_token(), anthropic_key());
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call(
                "Grep",
                json!({"pattern": github.clone(), "path": "src"}),
                ok(),
            ),
            call(
                "Bash",
                json!({"command": format!("curl -H 'x-api-key: {anthropic}' https://example.invalid")}),
                ok(),
            ),
            call(
                "Read",
                json!({"file_path": format!("keys/{github}.txt")}),
                ok(),
            ),
            summary(3, 3, 0),
        ],
    );
    let stored = serde_json::to_string(&result).unwrap();
    assert!(!stored.contains(&github), "{stored}");
    assert!(!stored.contains(&anthropic), "{stored}");
    assert!(!stored.contains("A1b2C3d4E5A1b2C3"), "{stored}");
    assert!(result.commands_run[0].command.contains("REDACTED"));
    assert!(result.commands_run[1].command.starts_with("curl "));
}

#[test]
fn a_long_input_is_redacted_then_clipped() {
    let mut result = WorkflowV2Result::accepted("raw");
    let long = format!("{} {}", "x".repeat(COMMAND_CHARS - 10), anthropic_key());
    record_tool_trace(
        &mut result,
        &[call("Grep", json!({"pattern": long}), Value::Null)],
    );
    let command = &result.commands_run[0].command;
    assert_eq!(command.chars().count(), COMMAND_CHARS);
    assert!(!command.contains("sk-ant"), "{command}");
}

#[test]
fn a_file_path_keeps_its_words_and_loses_only_a_credential_shaped_value() {
    let github = github_token();
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call("Read", json!({"file_path": "src/token_store.rs"}), ok()),
            call("Read", json!({"file_path": "config/secret_rules.md"}), ok()),
            call(
                "Read",
                json!({"file_path": format!("keys/{github}.txt")}),
                ok(),
            ),
            call("Grep", json!({"pattern": "secret", "path": "src"}), ok()),
            summary(4, 4, 0),
        ],
    );
    let read: Vec<&str> = result.files_read.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(read[..2], ["src/token_store.rs", "config/secret_rules.md"]);
    assert!(
        !read[2].contains(&github) && read[2].starts_with("keys/"),
        "{read:?}"
    );
    // Command text keeps the full redaction, words included.
    let grep = &result.commands_run[0].command;
    assert!(
        grep.starts_with("Grep ") && !grep.contains("secret"),
        "{grep}"
    );
}

#[test]
fn a_raw_result_says_its_lists_came_from_the_host_trace() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(&mut result, &[summary(0, 0, 0)]);
    assert_eq!(result.data["toolTrace"]["topLevelLists"], HOST_TRACE);
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(&mut result, &[]);
    assert_eq!(result.data["toolTrace"]["topLevelLists"], NOT_RECORDED);
}

#[test]
fn a_non_bash_call_stores_only_its_allow_listed_input() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call(
                "Write",
                json!({"file_path": "src/a.rs", "content": "PASSWORD=hunter2"}),
                ok(),
            ),
            call(
                "Edit",
                json!({"file_path": "src/b.rs", "old_string": "FOO_TOKEN=abc", "new_string": "x"}),
                ok(),
            ),
            call(
                "WebFetch",
                json!({"url": "https://example.invalid/v1?key=AIzaQQQQ", "prompt": "p"}),
                ok(),
            ),
            call(
                "Bash",
                json!({"command": "PASSWORD=hunter2 FOO_TOKEN=abc ./run"}),
                ok(),
            ),
            summary(4, 4, 0),
        ],
    );
    let stored = serde_json::to_string(&result).unwrap();
    for secret in ["hunter2", "=abc", "AIzaQQQQ", "content", "old_string"] {
        assert!(!stored.contains(secret), "{secret} in {stored}");
    }
    let commands: Vec<&str> = result
        .commands_run
        .iter()
        .map(|c| c.command.as_str())
        .collect();
    assert_eq!(commands[0], r#"Write {"file_path":"src/a.rs"}"#);
    assert_eq!(
        commands[2],
        r#"WebFetch {"url":"https://example.invalid/v1"}"#
    );
    // An assignment first: no program word is kept, only the count.
    assert!(commands[3] == " (2 args)", "{commands:?}");
}

#[test]
fn a_trace_missing_a_session_is_never_claimed_complete() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(&mut result, &[summary(0, 0, 0)]);
    assert_eq!(result.data["toolTrace"]["complete"], true);
    mark_incomplete(&mut result, &[]);
    assert_eq!(result.data["toolTrace"]["complete"], true);
    let reason = "a transient retry replaced a failed attempt".to_string();
    mark_incomplete(&mut result, &[reason.clone(), reason.clone()]);
    assert_eq!(result.data["toolTrace"]["complete"], false);
    assert_eq!(
        result.data["toolTrace"]["incompleteReasons"],
        json!([reason])
    );
}
