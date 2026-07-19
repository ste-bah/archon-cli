use archon_workflow::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2ImplementationInspector,
    WorkflowV2ImplementationStatus, WorkflowV2InspectionError, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus, WorkflowV2TaskFileStatus,
    WorkflowV2TaskRecord, WorkflowV2WorkItemKind,
};

#[test]
fn already_complete_task_noops_with_evidence_even_if_task_file_was_blocked() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T001", WorkflowV2TaskFileStatus::Blocked);
    let decision = inspector
        .inspect_task(&task, complete_noop_result("T001"))
        .expect("complete");

    assert_eq!(
        decision.implementation_status,
        WorkflowV2ImplementationStatus::Complete
    );
    assert!(decision.noop_result.is_some());
    assert!(decision.work_item.is_none());
}

#[test]
fn noop_without_files_read_is_rejected() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T001", WorkflowV2TaskFileStatus::Done);
    let mut result = complete_noop_result("T001");
    result.files_read.clear();

    let err = inspector
        .inspect_task(&task, result)
        .expect_err("missing files_read");

    assert_eq!(
        err,
        WorkflowV2InspectionError::NoopWithoutFilesRead("T001".to_string())
    );
}

#[test]
fn noop_without_command_evidence_is_rejected() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T001", WorkflowV2TaskFileStatus::Done);
    let mut result = complete_noop_result("T001");
    result.commands_run.clear();

    let err = inspector
        .inspect_task(&task, result)
        .expect_err("missing command evidence");

    assert_eq!(
        err,
        WorkflowV2InspectionError::NoopWithoutCommandEvidence("T001".to_string())
    );
}

#[test]
fn noop_cannot_hide_missing_acceptance_criteria() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let mut task = task_record("T001", WorkflowV2TaskFileStatus::Done);
    task.acceptance_criteria
        .push("second criterion".to_string());

    let err = inspector
        .inspect_task(&task, complete_noop_result("T001"))
        .expect_err("missing criterion");

    assert!(matches!(
        err,
        WorkflowV2InspectionError::NoopMissingAcceptanceCriterion { task_id, criterion }
            if task_id == "T001" && criterion == "second criterion"
    ));
}

#[test]
fn inspection_without_task_coverage_cannot_skip_required_task() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T001", WorkflowV2TaskFileStatus::NotStarted);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "inspected repository".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "read expected source files",
    ));

    let err = inspector
        .inspect_task(&task, result)
        .expect_err("missing coverage");

    assert_eq!(
        err,
        WorkflowV2InspectionError::MissingTaskCoverage("T001".to_string())
    );
}

#[test]
fn blocked_status_without_blocker_evidence_is_rejected() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T030", WorkflowV2TaskFileStatus::Blocked);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "blocked classification reported".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "inspected the target area",
    ));
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "T030".to_string(),
        status: WorkflowV2TaskCoverageStatus::Blocked,
        summary: "blocked but no blocker proof".to_string(),
        evidence: Vec::new(),
    });

    let err = inspector
        .inspect_task(&task, result)
        .expect_err("missing blocker evidence");

    assert_eq!(
        err,
        WorkflowV2InspectionError::BlockedWithoutEvidence("T030".to_string())
    );
}

#[test]
fn partial_task_creates_concrete_work_item() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T010", WorkflowV2TaskFileStatus::InProgress);
    let decision = inspector
        .inspect_task(&task, partial_result("T010"))
        .expect("partial");
    let item = decision.work_item.expect("work item");

    assert_eq!(
        decision.implementation_status,
        WorkflowV2ImplementationStatus::Partial
    );
    assert_eq!(item.kind, WorkflowV2WorkItemKind::Implementation);
    assert_eq!(item.task_id, "T010");
    assert_eq!(item.expected_target_files, vec!["src/example.rs"]);
    assert_eq!(item.acceptance_criteria, vec!["first criterion"]);
    assert_eq!(item.verification_commands, vec!["cargo test example"]);
    assert!(item.write_isolation_required);
}

#[test]
fn partial_task_without_verification_command_is_rejected() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let task = task_record("T010", WorkflowV2TaskFileStatus::InProgress);
    let mut result = partial_result("T010");
    result.commands_run.clear();

    let err = inspector
        .inspect_task(&task, result)
        .expect_err("missing verification command");

    assert_eq!(
        err,
        WorkflowV2InspectionError::WorkItemMissingVerificationCommands("T010".to_string())
    );
}

#[test]
fn unknown_task_creates_investigation_path() {
    let inspector = WorkflowV2ImplementationInspector::new();
    let mut task = task_record("T020", WorkflowV2TaskFileStatus::Unknown);
    task.candidate_target_files.clear();
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "could not determine implementation state".to_string(),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "source layout did not reveal an obvious target",
    ));
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "T020".to_string(),
        status: WorkflowV2TaskCoverageStatus::Unknown,
        summary: "inspect broader repository ownership before editing".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "candidate files are unknown",
        )],
    });

    let decision = inspector.inspect_task(&task, result).expect("unknown");
    let item = decision.work_item.expect("investigation item");

    assert_eq!(
        decision.implementation_status,
        WorkflowV2ImplementationStatus::Unknown
    );
    assert_eq!(item.kind, WorkflowV2WorkItemKind::Investigation);
    assert!(!item.write_isolation_required);
}

fn task_record(task_id: &str, status: WorkflowV2TaskFileStatus) -> WorkflowV2TaskRecord {
    WorkflowV2TaskRecord {
        task_id: task_id.to_string(),
        title: "Example task".to_string(),
        source_paths: vec!["tasks/TASK.md".to_string()],
        depends_on: vec!["T000".to_string()],
        acceptance_criteria: vec!["first criterion".to_string()],
        hard_rules: vec!["keep implementation generic".to_string()],
        candidate_target_files: vec!["src/example.rs".to_string()],
        status_from_task_file: status,
        implementation_status: WorkflowV2ImplementationStatus::Unknown,
    }
}

fn complete_noop_result(task_id: &str) -> WorkflowV2Result {
    let mut criterion_evidence = WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "first criterion is already implemented",
    );
    criterion_evidence.source = Some("acceptance:first criterion".to_string());

    WorkflowV2Result {
        status: WorkflowV2Status::Noop,
        summary: "task is already complete with proof".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "found existing module and command coverage",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Inspect,
            command: "rg first_criterion src/example.rs".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "symbol already exists".to_string(),
        }],
        files_read: vec![WorkflowV2FileRecord::new("src/example.rs")],
        task_coverage: vec![WorkflowV2TaskCoverage {
            task_id: task_id.to_string(),
            status: WorkflowV2TaskCoverageStatus::Noop,
            summary: "first criterion is already implemented".to_string(),
            evidence: vec![criterion_evidence],
        }],
        ..WorkflowV2Result::default()
    }
}

fn partial_result(task_id: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "task needs implementation work".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "existing code only covers part of the task",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test example".to_string(),
            status: WorkflowV2CommandStatus::Skipped,
            exit_code: None,
            output_summary: "verification command identified for implementation".to_string(),
        }],
        task_coverage: vec![WorkflowV2TaskCoverage {
            task_id: task_id.to_string(),
            status: WorkflowV2TaskCoverageStatus::Partial,
            summary: "implement the missing parser branch".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "missing branch is not present",
            )],
        }],
        ..WorkflowV2Result::default()
    }
}
