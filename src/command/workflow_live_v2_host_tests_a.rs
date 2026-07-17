use std::path::Path;

use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FinalReport, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2ResidualGap, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

use super::workflow_live_task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use super::workflow_live_v2_host::{
    artifact_path_exists, execute_local_host_call, reconcile_final_task_statuses,
    validated_completion_credit,
};

#[test]
fn inline_artifact_ref_is_not_reported_missing() {
    let temp = tempfile::tempdir().expect("tempdir");

    assert!(artifact_path_exists(temp.path(), "inline:data.items"));
}

#[test]
fn save_and_require_artifact_are_local_host_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let save = execution("artifact", WorkflowV2HostMethod::SaveArtifact, None);

    let saved = execute_local_host_call(&save, &store, None)
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
    let required = execute_local_host_call(&require, &store, None)
        .expect("require")
        .expect("local result");
    assert_eq!(required.status, WorkflowV2Status::Accepted);
}

#[test]
fn save_artifact_persists_source_data_not_call_envelope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let save = WorkflowV2CallExecution {
        input: serde_json::json!({
            "call_id": "artifact",
            "options": { "task": "save" },
            "source_data": { "artifact_paths": ["reports/a.json"], "marker": "source-only" }
        }),
        ..execution("artifact", WorkflowV2HostMethod::SaveArtifact, None)
    };

    let saved = execute_local_host_call(&save, &store, None)
        .expect("save")
        .expect("local result");
    let raw = std::fs::read_to_string(&saved.artifacts[0].path).expect("artifact body");
    let body: serde_json::Value = serde_json::from_str(&raw).expect("artifact json");

    assert_eq!(body["marker"], "source-only");
    assert!(body.get("call_id").is_none());
}

#[test]
fn require_artifact_without_explicit_paths_needs_review() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let require = WorkflowV2CallExecution {
        input: serde_json::json!({
            "call_id": "require-final-artifacts",
            "source_data": { "items": [] }
        }),
        ..execution(
            "require-final-artifacts",
            WorkflowV2HostMethod::RequireArtifact,
            None,
        )
    };

    let result = execute_local_host_call(&require, &store, None)
        .expect("require")
        .expect("local result");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.residual_gaps[0].id,
        "required_artifact_paths_missing_require-final-artifacts"
    );
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
    result.artifacts.push(archon_workflow::WorkflowV2Artifact {
        id: "coverage-history".to_string(),
        path: ".archon/trading-lab/data/coverage/history/not-written.json".to_string(),
        description: Some("optional display reference".to_string()),
    });
    result.data = serde_json::json!({
        "acceptance_criteria_results": [{
            "task_id": "TASK-TDL-080",
            "criterion": "Coverage proof is current and complete.",
            "status": "passed",
            "evidence_refs": [".archon/trading-lab/data/coverage/latest.json"]
        }]
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
    let report = execute_local_host_call(&final_report, &store, None)
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::Accepted);
    assert!(Path::new(&report.artifacts[0].path).exists());
}

#[test]
fn final_report_counts_project_relative_noop_evidence_and_ignores_placeholders() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp
        .path()
        .join("project-1")
        .join(".archon")
        .join("workflows")
        .join("wf-test")
        .join("v2");
    std::fs::create_dir_all(v2_root.join("results")).expect("results dir");
    let project_coverage = temp
        .path()
        .join("project-1")
        .join(".archon")
        .join("trading-lab")
        .join("data")
        .join("coverage");
    std::fs::create_dir_all(&project_coverage).expect("coverage dir");
    std::fs::write(project_coverage.join("latest.json"), "{}").expect("latest json");
    std::fs::write(project_coverage.join("latest.md"), "# coverage").expect("latest md");
    let store = WorkflowV2ResultStore::new(&v2_root);

    let upstream = execution(
        "noop-proof-verification-6",
        WorkflowV2HostMethod::Parallel,
        None,
    );
    let mut result = WorkflowV2Result::accepted("accepted noop coverage proof");
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-080".to_string(),
        status: WorkflowV2TaskCoverageStatus::Accepted,
        summary: "coverage matrix proof accepted".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Test,
            "coverage command tests passed",
        )],
    });
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test coverage_matrix".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "passed".to_string(),
    });
    let mut evidence = WorkflowV2TaskCompletionEvidence::new(
        "TASK-TDL-080",
        WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop,
        "noop-proof-verification-6",
        "NOOP-TDL-080",
        WorkflowV2Status::Accepted,
    );
    evidence.artifact_paths = vec![
        ".archon/trading-lab/data/coverage/latest.json".to_string(),
        ".archon/trading-lab/data/coverage/latest.md".to_string(),
        ".archon/trading-lab/data/coverage/history/<generated_at>.json".to_string(),
    ];
    evidence.evidence_refs = vec!["coverage command tests passed".to_string()];
    store
        .save_call_record(
            &WorkflowV2CallRecord::new(
                store.run_id(),
                upstream.call,
                1,
                "hash".to_string(),
                result,
                Vec::new(),
            )
            .with_completion_evidence(vec![evidence]),
        )
        .expect("record");

    let final_report = execution(
        "final",
        WorkflowV2HostMethod::FinalReport,
        Some("[noop-proof-verification-6]"),
    );
    let report = execute_local_host_call(&final_report, &store, Some(&task_universe_080()))
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::Accepted, "{report:#?}");
    assert_eq!(
        report.data["missing_tasks"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(
        report.data["artifacts"]
            .as_array()
            .is_some_and(|artifacts| artifacts.iter().all(|artifact| {
                artifact["id"] != "coverage-history"
            }))
    );
}
