use archon_workflow::{
    WorkflowV2Artifact, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2FinalReportBuilder,
    WorkflowV2FinalReportError, WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

#[test]
fn successful_report_includes_paths_commands_artifacts_and_status() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[accepted_result("T001")])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::Accepted);
    assert_eq!(report.paths.harness_path, "/run/workflow.js");
    assert_eq!(report.accepted_tasks, vec!["T001"]);
    assert_eq!(report.files_read[0].path, "src/lib.rs");
    assert_eq!(report.files_changed[0].path, "src/lib.rs");
    assert_eq!(report.commands_run[0].command, "cargo test focused");
    assert_eq!(report.tests_run[0].command, "cargo test focused");
    assert_eq!(report.artifacts[0].path, "artifacts/report.json");
    assert_eq!(report.review_findings, vec!["review passed"]);
}

#[test]
fn failed_required_task_prevents_success() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(
            paths(),
            &["T001".to_string()],
            &[coverage_result(
                "T001",
                WorkflowV2TaskCoverageStatus::Missing,
            )],
        )
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert_eq!(report.failed_tasks, vec!["T001"]);
}

#[test]
fn blocked_required_task_prevents_success() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[blocked_result("T001")])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert_eq!(report.blocked_tasks, vec!["T001"]);
    assert_eq!(report.residual_gaps[0].id, "external");
}

#[test]
fn missing_required_task_prevents_success() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(
            paths(),
            &["T001".to_string(), "T002".to_string()],
            &[accepted_result("T001")],
        )
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert_eq!(report.missing_tasks, vec!["T002"]);
}

#[test]
fn empty_work_with_valid_noop_proof_continues() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[noop_result("T001")])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::Accepted);
    assert_eq!(report.noop_tasks, vec!["T001"]);
    assert!(report.missing_tasks.is_empty());
}

#[test]
fn quoted_blocked_text_does_not_become_blocked_status() {
    let mut result = accepted_result("T001");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "The reviewed log quoted an older line: `status: blocked`.",
    ));

    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[result])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::Accepted);
    assert!(report.blocked_tasks.is_empty());
    assert!(
        report
            .review_findings
            .iter()
            .any(|finding| finding.contains("status: blocked"))
    );
}

#[test]
fn multi_task_wave_cannot_accept_single_task_proof() {
    let report = WorkflowV2FinalReportBuilder::new()
        .build(
            paths(),
            &[
                "T040".to_string(),
                "T050".to_string(),
                "T060".to_string(),
                "T070".to_string(),
            ],
            &[accepted_result("T040")],
        )
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert_eq!(report.accepted_tasks, vec!["T040"]);
    assert_eq!(
        report.missing_tasks,
        vec!["T050".to_string(), "T060".to_string(), "T070".to_string()]
    );
}

#[test]
fn missing_report_paths_are_rejected() {
    let mut paths = paths();
    paths.event_log_path.clear();

    let err = WorkflowV2FinalReportBuilder::new()
        .build(paths, &["T001".to_string()], &[accepted_result("T001")])
        .expect_err("missing path");

    assert_eq!(
        err,
        WorkflowV2FinalReportError::MissingPath {
            field: "event_log_path"
        }
    );
}

#[test]
fn residual_gap_blocks_even_when_task_coverage_is_accepted() {
    let mut result = accepted_result("T001");
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "gap".to_string(),
        description: "remaining documented gap".to_string(),
        severity: Some("blocking".to_string()),
    });

    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[result])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert_eq!(report.residual_gaps[0].id, "gap");
}

#[test]
fn unverified_test_claim_prevents_success() {
    let mut result = accepted_result("T001");
    result.commands_run.clear();

    let err = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[result])
        .expect_err("invalid result");

    assert!(matches!(
        err,
        WorkflowV2FinalReportError::InvalidResult { index: 0, .. }
    ));
}

#[test]
fn command_evidence_cannot_be_omitted_from_success() {
    let mut result = accepted_result("T001");
    result
        .evidence
        .retain(|evidence| evidence.kind != WorkflowV2EvidenceKind::Test);
    result.commands_run.clear();

    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &["T001".to_string()], &[result])
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
}

#[test]
fn hidden_needs_review_result_prevents_success() {
    let mut pending = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "review still required".to_string(),
        ..WorkflowV2Result::default()
    };
    pending.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "review failed",
    ));

    let report = WorkflowV2FinalReportBuilder::new()
        .build(
            paths(),
            &["T001".to_string()],
            &[accepted_result("T001"), pending],
        )
        .expect("report");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert!(report.failed_tasks.is_empty());
}

#[test]
fn cross_cutting_review_blocker_does_not_double_count_accepted_tasks() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/d29_blocked_review_conflict.json"))
            .expect("fixture");
    let required = fixture["required_task_ids"]
        .as_array()
        .expect("required tasks")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut review = coverage_result("T001", WorkflowV2TaskCoverageStatus::Accepted);
    review.residual_gaps.push(WorkflowV2ResidualGap {
        id: fixture["review_gap"]["id"].as_str().unwrap().to_string(),
        description: fixture["review_gap"]["description"]
            .as_str()
            .unwrap()
            .to_string(),
        severity: Some("critical".to_string()),
    });

    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths(), &required, &[accepted_result("T001"), review])
        .expect("report");

    assert_eq!(report.accepted_tasks, vec!["T001"]);
    assert!(report.failed_tasks.is_empty());
    assert!(report.blocked_tasks.is_empty());
    assert_eq!(
        report.review_blockers[0].id,
        "GAP-REVIEW-ACCEPTANCE-CONFLICT"
    );
    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
}

fn paths() -> WorkflowV2ReportPaths {
    WorkflowV2ReportPaths {
        harness_path: "/run/workflow.js".to_string(),
        run_state_path: "/run/state.json".to_string(),
        event_log_path: "/run/events.jsonl".to_string(),
    }
}

fn accepted_result(task_id: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "task accepted".to_string(),
        evidence: vec![
            WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Test, "focused tests passed"),
            WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Review, "review passed"),
        ],
        artifacts: vec![WorkflowV2Artifact {
            id: "report".to_string(),
            path: "artifacts/report.json".to_string(),
            description: Some("acceptance report".to_string()),
        }],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test focused".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "passed".to_string(),
        }],
        files_read: vec![WorkflowV2FileRecord::new("src/lib.rs")],
        files_changed: vec![WorkflowV2FileRecord::new("src/lib.rs")],
        task_coverage: vec![coverage(task_id, WorkflowV2TaskCoverageStatus::Accepted)],
        ..WorkflowV2Result::default()
    }
}

fn noop_result(task_id: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Noop,
        summary: "task already implemented".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "inspected existing implementation and task acceptance criteria",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Inspect,
            command: "rg T001 tasks src".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "existing implementation satisfies the task".to_string(),
        }],
        files_read: vec![WorkflowV2FileRecord::new("src/lib.rs")],
        task_coverage: vec![coverage(task_id, WorkflowV2TaskCoverageStatus::Noop)],
        ..WorkflowV2Result::default()
    }
}

fn coverage_result(task_id: &str, status: WorkflowV2TaskCoverageStatus) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "task coverage is incomplete".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "inspected task state",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Inspect,
            command: "rg task src".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "missing".to_string(),
        }],
        task_coverage: vec![coverage(task_id, status)],
        ..WorkflowV2Result::default()
    }
}

fn blocked_result(task_id: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "external blocker".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "external dependency unavailable",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Inspect,
            command: "check external dependency".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "unavailable".to_string(),
        }],
        task_coverage: vec![coverage(task_id, WorkflowV2TaskCoverageStatus::Blocked)],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: "external".to_string(),
            description: "external dependency unavailable".to_string(),
            severity: Some("blocking".to_string()),
        }],
        ..WorkflowV2Result::default()
    }
}

fn coverage(task_id: &str, status: WorkflowV2TaskCoverageStatus) -> WorkflowV2TaskCoverage {
    WorkflowV2TaskCoverage {
        task_id: task_id.to_string(),
        status,
        summary: "coverage summary".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "coverage evidence",
        )],
    }
}
