use super::*;

fn call(name: &str, input: Value, output: Value) -> WorkflowAgentToolUse {
    WorkflowAgentToolUse {
        tool_name: name.to_string(),
        input,
        output,
    }
}

#[test]
fn reads_become_files_read_once_and_other_calls_become_commands() {
    let mut result = WorkflowV2Result::accepted("raw");
    record_tool_trace(
        &mut result,
        &[
            call(
                "Read",
                json!({"file_path": "a.rs"}),
                json!({"is_error": false}),
            ),
            call(
                "Read",
                json!({"file_path": "a.rs"}),
                json!({"is_error": false}),
            ),
            call(
                "Read",
                json!({"file_path": "gone.rs"}),
                json!({"is_error": true}),
            ),
            call(
                "Glob",
                json!({"pattern": "**/*.rs"}),
                json!({"is_error": false}),
            ),
            call("Bash", json!({"command": "ls"}), Value::Null),
        ],
    );
    assert_eq!(result.files_read.len(), 1);
    assert_eq!(result.files_read[0].path, "a.rs");
    let commands: Vec<_> = result
        .commands_run
        .iter()
        .map(|c| (c.command.as_str(), c.status))
        .collect();
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(commands[0].0.starts_with("Read ") && commands[0].1 == WorkflowV2CommandStatus::Failed);
    assert!(commands[1].0.starts_with("Glob ") && commands[1].0.contains("**/*.rs"));
    assert_eq!(commands[2], ("ls", WorkflowV2CommandStatus::Skipped));
    assert_eq!(result.data["toolTrace"]["toolCalls"], 5);
}

#[test]
fn an_empty_trace_is_marked_not_recorded_and_keeps_existing_data() {
    let mut result = WorkflowV2Result::accepted("raw");
    result.data = json!({"content": "x"});
    record_tool_trace(&mut result, &[]);
    assert_eq!(result.data["content"], "x");
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
fn a_long_input_is_clipped() {
    let mut result = WorkflowV2Result::accepted("raw");
    let long = "x".repeat(1_000);
    record_tool_trace(
        &mut result,
        &[call("Bash", json!({"command": long}), Value::Null)],
    );
    assert_eq!(
        result.commands_run[0].command.chars().count(),
        COMMAND_CHARS
    );
}
