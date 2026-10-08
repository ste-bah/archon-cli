//! Issue 276: with a captured session trace, a no-op proof takes as proof of
//! inspection only the claimed reads the trace confirms.
use archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
use archon_workflow::v2::tool_trace::record_structured_trace;
use archon_workflow::{
    WorkflowAgentToolUse, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord,
    WorkflowV2ImplementationInspector, WorkflowV2ImplementationStatus, WorkflowV2InspectionError,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2TaskFileStatus, WorkflowV2TaskRecord,
};
use serde_json::{Value, json};

fn noop_claiming(path: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("already implemented");
    result.status = WorkflowV2Status::Noop;
    result.files_read.push(WorkflowV2FileRecord::new(path));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test -p demo".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "test result: ok. 1 passed".to_string(),
        pre_existing: false,
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "the behaviour exists",
    ));
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "T001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "already implemented".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read the module and ran its tests",
        )],
    });
    result
}

fn task() -> WorkflowV2TaskRecord {
    WorkflowV2TaskRecord {
        task_id: "T001".to_string(),
        title: "Example task".to_string(),
        source_paths: vec!["tasks/TASK.md".to_string()],
        depends_on: Vec::new(),
        acceptance_criteria: Vec::new(),
        hard_rules: Vec::new(),
        candidate_target_files: vec!["src/example.rs".to_string()],
        status_from_task_file: WorkflowV2TaskFileStatus::Done,
        implementation_status: WorkflowV2ImplementationStatus::Unknown,
    }
}

fn trace_reading(path: &str) -> Vec<WorkflowAgentToolUse> {
    vec![
        WorkflowAgentToolUse {
            tool_name: "Read".to_string(),
            input: json!({"file_path": path}),
            output: json!({"is_error": false}),
        },
        WorkflowAgentToolUse {
            tool_name: TOOL_TRACE_SUMMARY_NAME.to_string(),
            input: json!({"calls": 1, "kept": 1, "dropped": 0, "inputs_truncated": 0}),
            output: Value::Null,
        },
    ]
}

#[test]
fn a_claimed_read_the_trace_confirms_is_proof() {
    let mut result = noop_claiming("src/example.rs");
    record_structured_trace(&mut result, Some(&trace_reading("/repo/src/example.rs")));
    let decision = WorkflowV2ImplementationInspector::new()
        .inspect_task(&task(), result)
        .expect("confirmed claim is proof");
    let proof = &decision.noop_result.expect("noop").data["noopProof"];
    assert_eq!(proof["filesRead"], "trace_confirmed", "{proof}");
    assert_eq!(proof["confirmed"], 1);
}

#[test]
fn a_claimed_read_the_trace_never_saw_is_not_proof() {
    let mut result = noop_claiming("src/example.rs");
    record_structured_trace(&mut result, Some(&trace_reading("/repo/src/other.rs")));
    let err = WorkflowV2ImplementationInspector::new()
        .inspect_task(&task(), result)
        .expect_err("unmatched claim is not proof");
    assert_eq!(
        err,
        WorkflowV2InspectionError::NoopFilesReadNotObserved {
            task_id: "T001".to_string(),
            unmatched: "src/example.rs".to_string(),
        }
    );
    assert!(
        err.to_string().contains("unmatched: src/example.rs"),
        "{err}"
    );
}

#[test]
fn without_a_trace_the_claims_still_count_and_are_marked_agent_reported() {
    for trace in [None, Some(&[][..])] {
        let mut result = noop_claiming("src/example.rs");
        record_structured_trace(&mut result, trace);
        let decision = WorkflowV2ImplementationInspector::new()
            .inspect_task(&task(), result)
            .expect("today's behaviour without a trace");
        let proof = &decision.noop_result.expect("noop").data["noopProof"];
        assert_eq!(proof["filesRead"], "agent_reported", "{proof}");
    }
    // A result that never went through the host trace at all.
    let decision = WorkflowV2ImplementationInspector::new()
        .inspect_task(&task(), noop_claiming("src/example.rs"))
        .expect("unchanged");
    assert_eq!(
        decision.noop_result.unwrap().data["noopProof"]["filesRead"],
        "agent_reported"
    );
}
