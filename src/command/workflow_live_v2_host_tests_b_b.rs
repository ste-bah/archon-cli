use super::*;

#[test]
fn blocked_review_final_report_collects_nested_verification_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let mut review_verification = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "review verification found residual gaps".to_string(),
        ..WorkflowV2Result::default()
    };
    review_verification.residual_gaps.push(WorkflowV2ResidualGap {
        id: "review_verification_gap".to_string(),
        description: "coverage artifact is incomplete".to_string(),
        severity: Some("blocking".to_string()),
    });
    let final_report = WorkflowV2CallExecution {
        input: serde_json::json!({
            "status": "needs_review",
            "inputs": { "reviewVerification": review_verification },
        }),
        ..execution(
            "blocked-review-verification-failed-1",
            WorkflowV2HostMethod::FinalReport,
            None,
        )
    };

    let report = execute_local_host_call(&final_report, &store, Some(&task_universe_010()))
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert!(
        report.data["residual_gaps"]
            .as_array()
            .is_some_and(|gaps| gaps.iter().any(|gap| gap["id"] == "review_verification_gap")),
        "{:#?}",
        report.data
    );
    assert_eq!(
        report.data["blocker"]["call_id"],
        serde_json::json!("blocked-review-verification-failed-1")
    );
}

#[test]
fn blocked_final_report_ignores_accepted_metadata_without_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let metadata_only = WorkflowV2Result::accepted("verification repair plan accepted");
    let mut failed_verification = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "focused verification failed".to_string(),
        ..WorkflowV2Result::default()
    };
    failed_verification.residual_gaps.push(WorkflowV2ResidualGap {
        id: "focused_test_failed".to_string(),
        description: "focused test ran and failed".to_string(),
        severity: Some("high".to_string()),
    });
    let final_report = WorkflowV2CallExecution {
        input: serde_json::json!({
            "status": "needs_review",
            "inputs": {
                "repairPlan": metadata_only,
                "verification": failed_verification,
            },
        }),
        ..execution(
            "blocked-verification-failed-1",
            WorkflowV2HostMethod::FinalReport,
            None,
        )
    };

    let report = execute_local_host_call(&final_report, &store, Some(&task_universe_010()))
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert!(
        report.data["residual_gaps"]
            .as_array()
            .is_some_and(|gaps| gaps.iter().any(|gap| gap["id"] == "focused_test_failed")),
        "{:#?}",
        report.data
    );
}

#[test]
fn final_accounting_removes_cross_cutting_task_status_collisions() {
    let mut report: WorkflowV2FinalReport = serde_json::from_value(serde_json::json!({
        "status": "needs_review",
        "paths": {"harness_path":"h", "run_state_path":"s", "event_log_path":"e"},
        "task_coverage": [], "files_read": [], "files_changed": [],
        "commands_run": [], "tests_run": [], "review_findings": [],
        "remediation_actions": [], "artifacts": [],
        "accepted_tasks": ["TASK-1"], "noop_tasks": [],
        "failed_tasks": ["GAP-REVIEW", "TASK-1"],
        "blocked_tasks": ["TASK-1"], "missing_tasks": [],
        "review_blockers": [],
        "residual_gaps": [{
            "id": "GAP-REVIEW", "description": "project-level review blocker",
            "severity": "blocking"
        }]
    }))
    .expect("report fixture");

    reconcile_final_task_statuses(&mut report, &["TASK-1".to_string()]);

    assert_eq!(report.accepted_tasks, vec!["TASK-1"]);
    assert!(report.failed_tasks.is_empty());
    assert!(report.blocked_tasks.is_empty());
    assert_eq!(report.review_blockers[0].id, "GAP-REVIEW");
}
