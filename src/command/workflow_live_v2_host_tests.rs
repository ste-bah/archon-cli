use std::path::Path;

use archon_workflow::{
    WorkflowV2CallExecution, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

use super::workflow_live_v2_host::execute_local_host_call;

#[test]
fn save_and_require_artifact_are_local_host_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let save = execution("artifact", WorkflowV2HostMethod::SaveArtifact, None);

    let saved = execute_local_host_call(&save, &store)
        .expect("save")
        .expect("local result");
    assert_eq!(saved.status, WorkflowV2Status::Accepted);
    assert!(Path::new(&saved.artifacts[0].path).exists());

    let require = WorkflowV2CallExecution {
        input: serde_json::json!({ "path": saved.artifacts[0].path }),
        ..execution(
            "require-artifact",
            WorkflowV2HostMethod::RequireArtifact,
            None,
        )
    };
    let required = execute_local_host_call(&require, &store)
        .expect("require")
        .expect("local result");
    assert_eq!(required.status, WorkflowV2Status::Accepted);
}

#[test]
fn final_report_is_derived_from_typed_inputs_and_saved() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let upstream = execution("implement", WorkflowV2HostMethod::Agent, None);
    let mut result = WorkflowV2Result::accepted("implemented one task");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "changed concrete files",
    ));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "pytest tests/test_one.py::test_one".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "passed".to_string(),
    });
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "T001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Accepted,
        summary: "done".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "accepted evidence",
        )],
    });
    store
        .save_call_record(&archon_workflow::WorkflowV2CallRecord::new(
            store.run_id(),
            upstream.call,
            1,
            "hash".to_string(),
            result,
            Vec::new(),
        ))
        .expect("record");

    let final_report = execution(
        "final",
        WorkflowV2HostMethod::FinalReport,
        Some("[implement]"),
    );
    let report = execute_local_host_call(&final_report, &store)
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::Accepted);
    assert!(Path::new(&report.artifacts[0].path).exists());
}

#[test]
fn missing_required_artifact_becomes_review_choice_not_blocked_engine_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let missing = temp.path().join("missing.json");
    let require = WorkflowV2CallExecution {
        input: serde_json::json!({ "path": missing }),
        ..execution(
            "require-artifact",
            WorkflowV2HostMethod::RequireArtifact,
            None,
        )
    };

    let result = execute_local_host_call(&require, &store)
        .expect("require")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.residual_gaps[0].severity.as_deref(),
        Some("remediation")
    );
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| !choices.is_empty())
    );
}

#[test]
fn quality_gate_returns_review_choices_for_non_accepted_inputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let upstream = execution("audit", WorkflowV2HostMethod::Agent, None);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "audit found gaps".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "review evidence",
    ));
    store
        .save_call_record(&archon_workflow::WorkflowV2CallRecord::new(
            store.run_id(),
            upstream.call,
            1,
            "hash".to_string(),
            result,
            Vec::new(),
        ))
        .expect("record");

    let gate = execution(
        "quality",
        WorkflowV2HostMethod::QualityGate,
        Some("[audit]"),
    );
    let result = execute_local_host_call(&gate, &store)
        .expect("quality")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.residual_gaps[0].severity.as_deref(), Some("review"));
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| choices.iter().any(|choice| choice
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some("run_remediation")))
    );
}

#[test]
fn human_gate_is_never_auto_accepted_and_returns_choices() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let gate = execution("approval", WorkflowV2HostMethod::HumanGate, None);

    let result = execute_local_host_call(&gate, &store)
        .expect("gate")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(!result.residual_gaps.is_empty());
    assert!(
        result
            .data
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|choices| choices.iter().any(|choice| choice
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some("approve_continue")))
    );
}

fn execution(
    id: &str,
    method: WorkflowV2HostMethod,
    source: Option<&str>,
) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method,
            write_mode: None,
            options: WorkflowV2HostOptions {
                source: source.map(str::to_string),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "payload": id }),
        depends_on: Vec::new(),
    }
}
