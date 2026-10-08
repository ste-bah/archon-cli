use super::*;
use crate::v2::result::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2FileRecord,
};
use crate::v2::tool_trace::NOT_RECORDED;

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

fn summary(calls: u64, kept: u64, cut: u64) -> WorkflowAgentToolUse {
    call(
        TOOL_TRACE_SUMMARY_NAME,
        json!({"calls": calls, "kept": kept, "dropped": calls - kept, "inputs_truncated": cut}),
        Value::Null,
    )
}

fn agent_reported_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("structured");
    result
        .files_read
        .push(WorkflowV2FileRecord::new("agent/said.rs"));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "test result: ok. 1 passed".to_string(),
        pre_existing: false,
    });
    result
}

#[test]
fn a_structured_result_keeps_its_lists_and_carries_the_observed_trace_beside_them() {
    let mut result = agent_reported_result();
    result.data = json!({"toolTrace": "agent forged", "other": 1});
    let trace = [
        call("Read", json!({"file_path": "src/a.rs"}), ok()),
        call("Bash", json!({"command": "ls"}), ok()),
        summary(2, 2, 0),
    ];
    record_structured_trace(&mut result, Some(&trace));
    assert_eq!(result.files_read[0].path, "agent/said.rs");
    assert_eq!(result.commands_run.len(), 1);
    assert_eq!(result.commands_run[0].exit_code, Some(0));
    assert!(
        result.evidence.is_empty(),
        "gates count evidence: none is added"
    );
    let marker = &result.data["toolTrace"];
    assert_eq!(marker["topLevelLists"], AGENT_REPORTED);
    assert_eq!(marker["recorded"], true);
    assert_eq!(marker["filesRead"][0]["path"], "src/a.rs");
    assert!(
        marker["commandsRun"][0]["command"]
            .as_str()
            .is_some_and(|command| command.starts_with("ls (0 args")),
        "{marker}"
    );
    assert_eq!(result.data["other"], 1);
}

#[test]
fn a_structured_result_without_a_trace_is_marked_agent_reported_and_not_recorded() {
    for trace in [None, Some(&[][..])] {
        let mut result = agent_reported_result();
        record_structured_trace(&mut result, trace);
        let marker = &result.data["toolTrace"];
        assert_eq!(marker["topLevelLists"], AGENT_REPORTED);
        assert_eq!(marker["recorded"], false);
        assert_eq!(marker["filesRead"], NOT_RECORDED);
        assert_eq!(result.files_read.len(), 1);
        assert!(result.evidence.is_empty());
    }
}

#[test]
fn structured_data_that_is_not_an_object_is_left_as_written() {
    let mut result = agent_reported_result();
    result.data = json!(["agent", "list"]);
    record_structured_trace(&mut result, None);
    assert_eq!(result.data, json!(["agent", "list"]));
}

#[test]
fn session_traces_merge_in_order_and_sum_their_summaries() {
    let first = vec![
        call("Read", json!({"file_path": "a"}), ok()),
        summary(1, 1, 0),
    ];
    let repair = vec![
        call("Bash", json!({"command": "ls"}), ok()),
        summary(3, 1, 1),
    ];
    let merged = merge_session_traces(vec![first.clone(), repair]).unwrap();
    let names: Vec<_> = merged.iter().map(|t| t.tool_name.as_str()).collect();
    assert_eq!(names, ["Read", "Bash", TOOL_TRACE_SUMMARY_NAME]);
    let totals = &merged[2].input;
    assert_eq!(
        (
            &totals["calls"],
            &totals["kept"],
            &totals["dropped"],
            &totals["inputs_truncated"]
        ),
        (&json!(4), &json!(2), &json!(2), &json!(1))
    );
    // One session not captured: no summary, so never claimed complete.
    let uncaptured = vec![call("Grep", json!({"pattern": "x"}), ok())];
    let merged = merge_session_traces(vec![first, uncaptured]).unwrap();
    assert!(
        merged
            .iter()
            .all(|t| t.tool_name != TOOL_TRACE_SUMMARY_NAME)
    );
    assert!(merge_session_traces(Vec::new()).is_none());
}

#[test]
fn a_claimed_read_the_trace_never_saw_is_named() {
    let mut result = agent_reported_result();
    result
        .files_read
        .push(WorkflowV2FileRecord::new("src/a.rs"));
    let trace = [
        call("Read", json!({"file_path": "/repo/src/a.rs"}), ok()),
        summary(1, 1, 0),
    ];
    record_structured_trace(&mut result, Some(&trace));
    let check = &result.data["toolTrace"]["claimCheck"];
    assert_eq!(check["claimsMatchTrace"], false, "{check}");
    assert_eq!(check["filesRead"]["unobserved"], json!(["agent/said.rs"]));
    assert_eq!(check["filesRead"]["unobservedCount"], 1);
    assert_eq!(check["filesRead"]["traceComplete"], true);
    crate::v2::tool_trace::mark_incomplete(&mut result, &["lost session".to_string()]);
    let check = &result.data["toolTrace"]["claimCheck"];
    assert_eq!(check["filesRead"]["traceComplete"], false);
}

#[test]
fn claims_that_match_observed_reads_pass_the_check() {
    let mut result = WorkflowV2Result::accepted("structured");
    result
        .files_read
        .push(WorkflowV2FileRecord::new("./src/a.rs"));
    let trace = [
        call("Read", json!({"file_path": "/repo/src/a.rs"}), ok()),
        summary(1, 1, 0),
    ];
    record_structured_trace(&mut result, Some(&trace));
    assert_eq!(
        result.data["toolTrace"]["claimCheck"]["claimsMatchTrace"],
        true
    );
    assert!(!same_file("/repo/xsrc/a.rs", "src/a.rs"));
}

#[test]
fn the_observed_lists_kept_in_data_are_bounded() {
    let mut result = agent_reported_result();
    let mut trace: Vec<_> = (0..100)
        .map(|i| call("Read", json!({"file_path": format!("f{i}.rs")}), ok()))
        .collect();
    trace.push(summary(100, 100, 0));
    record_structured_trace(&mut result, Some(&trace));
    let marker = &result.data["toolTrace"];
    assert_eq!(
        marker["filesRead"].as_array().unwrap().len(),
        MAX_OBSERVED_IN_DATA
    );
    assert_eq!(marker["filesReadTotal"], 100);
}
