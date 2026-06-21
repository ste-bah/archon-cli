use archon_workflow::v2::{
    WorkflowV2Artifact, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2ResidualGap,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2ValidationError,
};

fn inspection_evidence() -> WorkflowV2Evidence {
    WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "inspected target files and acceptance criteria",
    )
}

#[test]
fn valid_accepted_result_passes() {
    let mut result = WorkflowV2Result::accepted("implemented requested slice");
    result.evidence.push(inspection_evidence());
    result.files_changed.push(WorkflowV2FileRecord::new(
        "crates/archon-workflow/src/v2/result.rs",
    ));

    result.validate().unwrap();
}

#[test]
fn valid_noop_result_requires_and_accepts_proof() {
    let mut result = WorkflowV2Result::noop("task already implemented");
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-CWF-001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "V2 boundary already exists".to_string(),
        evidence: vec![inspection_evidence()],
    });

    result.validate().unwrap();
}

#[test]
fn accepted_without_evidence_is_rejected() {
    let result = WorkflowV2Result::accepted("trust me");

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::MissingEvidence(WorkflowV2Status::Accepted)
    );
}

#[test]
fn blocked_without_blocker_is_rejected() {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "cannot continue".to_string(),
        ..WorkflowV2Result::default()
    };

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::MissingBlocker
    );
}

#[test]
fn blocked_with_residual_gap_passes() {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "external service unavailable".to_string(),
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: "gap-provider".to_string(),
            description: "provider credentials unavailable".to_string(),
            severity: Some("blocking".to_string()),
        }],
        ..WorkflowV2Result::default()
    };

    result.validate().unwrap();
}

#[test]
fn blocked_with_empty_residual_gap_is_rejected() {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "external service unavailable".to_string(),
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: " ".to_string(),
            description: " ".to_string(),
            severity: Some("blocking".to_string()),
        }],
        ..WorkflowV2Result::default()
    };

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::MissingBlocker
    );
}

#[test]
fn blocked_with_empty_blocker_evidence_is_rejected() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "blocked".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        " ",
    ));

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::MissingBlocker
    );
}

#[test]
fn test_evidence_without_successful_test_command_is_rejected() {
    let mut result = WorkflowV2Result::accepted("tests passed");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "focused tests passed",
    ));

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::TestEvidenceWithoutCommand
    );
}

#[test]
fn test_evidence_with_successful_test_command_passes() {
    let mut result = WorkflowV2Result::accepted("tests passed");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "focused tests passed",
    ));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-workflow --test v2_result_contracts".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "tests passed".to_string(),
    });

    result.validate().unwrap();
}

#[test]
fn failed_test_evidence_with_failed_command_passes_for_remediation() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "focused test failed and needs remediation".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "focused test failed",
    ));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-workflow --test v2_remediation".to_string(),
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: "one focused test failed".to_string(),
    });

    result.validate().unwrap();
}

#[test]
fn changed_file_claim_requires_path() {
    let mut result = WorkflowV2Result::accepted("changed files");
    result.evidence.push(inspection_evidence());
    result.files_changed.push(WorkflowV2FileRecord::new(" "));

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::EmptyChangedFilePath(0)
    );
}

#[test]
fn artifact_claim_requires_path() {
    let mut result = WorkflowV2Result::accepted("artifact emitted");
    result.evidence.push(inspection_evidence());
    result.artifacts.push(WorkflowV2Artifact {
        id: "artifact".to_string(),
        path: " ".to_string(),
        description: None,
    });

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::EmptyArtifactPath(0)
    );
}

#[test]
fn malformed_task_coverage_is_rejected() {
    let mut result = WorkflowV2Result::noop("no-op coverage");
    result.evidence.push(inspection_evidence());
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: String::new(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "bad coverage".to_string(),
        evidence: vec![inspection_evidence()],
    });

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::EmptyTaskId(0)
    );
}

#[test]
fn task_coverage_requires_summary() {
    let mut result = WorkflowV2Result::noop("no-op coverage");
    result.evidence.push(inspection_evidence());
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-CWF-001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: " ".to_string(),
        evidence: vec![inspection_evidence()],
    });

    assert_eq!(
        result.validate().unwrap_err(),
        WorkflowV2ValidationError::EmptyTaskCoverageSummary(0)
    );
}

#[test]
fn markdown_only_output_cannot_satisfy_typed_result() {
    let err = serde_json::from_str::<WorkflowV2Result>("\"looks good\"").unwrap_err();
    assert!(err.is_data());
}

#[test]
fn completed_with_gaps_result_status_maps_to_needs_review() {
    let result: WorkflowV2Result = serde_json::from_value(serde_json::json!({
        "status": "completed_with_gaps",
        "summary": "review found remaining gaps"
    }))
    .expect("status alias should parse");

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    result.validate().unwrap();
}

#[test]
fn completed_with_gaps_task_coverage_maps_to_partial() {
    let coverage: WorkflowV2TaskCoverage = serde_json::from_value(serde_json::json!({
        "task_id": "TASK-CWF-001",
        "status": "completed_with_gaps",
        "summary": "implemented with residual review findings"
    }))
    .expect("task coverage status alias should parse");

    assert_eq!(coverage.status, WorkflowV2TaskCoverageStatus::Partial);
}
