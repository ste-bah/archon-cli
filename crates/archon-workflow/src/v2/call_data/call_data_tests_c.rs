use super::*;

#[test]
fn focused_verification_duplicate_cargo_harness_pass_is_accepted() {
    let first: WorkflowV2BranchOutcome =
        serde_json::from_str(archon_test_support::fixtures::WFCD824_VERIFICATION_WAVE_1_3_CHECK_1)
            .expect("first fixture");
    let second: WorkflowV2BranchOutcome =
        serde_json::from_str(archon_test_support::fixtures::WFCD824_VERIFICATION_WAVE_1_3_CHECK_2)
            .expect("second fixture");

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1-3"),
        report(vec![first, second]),
    );

    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let outcomes = result.data["outcomes"].as_array().expect("outcomes");
    assert_eq!(outcomes.len(), 2);
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome["status"] == "accepted"),
        "{outcomes:#?}"
    );
    assert!(
        outcomes.iter().all(
            |outcome| outcome["completion_evidence"]
                .as_array()
                .is_some_and(
                    |items| items.iter().any(|item| item["task_id"] == "TASK-TDL-010"
                        && item["source_fingerprint"] == "focused-verification-evidence-v2")
                )
        ),
        "{outcomes:#?}"
    );
}

#[test]
fn focused_verification_failed_tests_require_write_remediation() {
    let failed: WorkflowV2BranchOutcome = serde_json::from_str(
        archon_test_support::fixtures::WF2D24_VERIFICATION_WAVE_1_3_DATA_STORE_FAILED,
    )
    .expect("fixture");

    let result =
        result_from_fanout_report(&fanout_call("verification-wave-1-3"), report(vec![failed]));

    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    let outcome = &result.data["outcomes"].as_array().expect("outcomes")[0];
    assert_eq!(
        outcome["result"]["data"]["verification_failure_class"],
        "actionable_implementation_failure"
    );
    assert_eq!(
        outcome["result"]["data"]["verification_failure_next_action"],
        "write_remediation"
    );
    assert_eq!(outcome["canonical_task_ids"][0], "TASK-TDL-010");
    assert_eq!(
        outcome["result"]["data"]["source_item_id"],
        "impl-TASK-TDL-010-registry-schema-v2"
    );
}

#[test]
fn focused_verification_zero_matched_tests_stays_needs_review() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "focused command exited 0 but no target test matched".to_string(),
        ..WorkflowV2Result::default()
    };
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test missing_target -- --exact".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary:
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1163 filtered out"
                .to_string(),
    });
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "zero-tests".to_string(),
        description: "exactly one targeted test expected but zero matched".to_string(),
        severity: Some("medium".to_string()),
    });
    result.data = serde_json::json!({ "canonical_task_ids": ["TASK-TDL-010"] });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "zero-match",
            WorkflowV2Status::NeedsReview,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["outcomes"][0]["status"], "needs_review");
}

#[test]
fn accepted_verification_skipped_command_is_not_completion_command_proof() {
    let mut result = WorkflowV2Result::accepted("verification evidence recorded");
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test focused_check".to_string(),
        status: crate::WorkflowV2CommandStatus::Skipped,
        exit_code: Some(0),
        output_summary: "not executed in this stage".to_string(),
    });
    result.task_coverage.push(crate::WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-010".to_string(),
        status: crate::WorkflowV2TaskCoverageStatus::Accepted,
        summary: "verification accepted from other concrete evidence".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Test,
            "focused verification evidence",
        )],
    });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "verify-TASK-TDL-010",
            WorkflowV2Status::Accepted,
            Some(result),
            None,
        )]),
    );

    // A skipped command is not execution: with no succeeded command at all,
    // the zero-command backstop demotes the acceptance outright — prose
    // coverage evidence alone must never verify a task.
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["outcomes"][0]["status"], "needs_review");
    assert_eq!(
        result.data["outcomes"][0]["result"]["data"]["zero_command_verification"],
        true
    );
}

#[test]
fn focused_verification_nonzero_exit_stays_failed() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "targeted test failed".to_string(),
        ..WorkflowV2Result::default()
    };
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test failing_target -- --exact".to_string(),
        status: crate::WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: "test result: failed. 0 passed; 1 failed".to_string(),
    });
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "target-failed".to_string(),
        description: "duplicate targeted test output was not the issue; the command failed"
            .to_string(),
        severity: Some("blocking".to_string()),
    });
    result.data = serde_json::json!({ "canonical_task_ids": ["TASK-TDL-010"] });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "failed-target",
            WorkflowV2Status::Failed,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["outcomes"][0]["status"], "failed");
}

pub(super) fn fanout_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options: Default::default(),
    }
}

pub(super) fn implementation_fanout_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    }
}

pub(super) fn report(outcomes: Vec<WorkflowV2BranchOutcome>) -> WorkflowV2FanoutReport {
    WorkflowV2FanoutReport {
        outcomes,
        max_parallelism: 8,
        peak_parallelism: 2,
        cancelled: false,
    }
}

pub(super) fn accepted_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted(format!("{id} accepted"));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "branch inspected concrete input",
    ));
    outcome(id, WorkflowV2Status::Accepted, Some(result), None)
}

pub(super) fn review_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!("{id} needs review"),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "branch found a concrete review item",
    ));
    outcome(id, WorkflowV2Status::NeedsReview, Some(result), None)
}

pub(super) fn blocked_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: format!("{id} blocked"),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        "branch found a concrete blocker",
    ));
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: format!("{id}-gap"),
        description: "missing concrete artifact".to_string(),
        severity: Some("blocking".to_string()),
    });
    outcome(id, WorkflowV2Status::Blocked, Some(result), None)
}

pub(super) fn failed_error_outcome(id: &str, error: &str) -> WorkflowV2BranchOutcome {
    outcome(id, WorkflowV2Status::Failed, None, Some(error.to_string()))
}

pub(super) fn outcome(
    id: &str,
    status: WorkflowV2Status,
    result: Option<WorkflowV2Result>,
    error: Option<String>,
) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: id.to_string(),
        role: "researcher".to_string(),
        status,
        result,
        error,
        failure_kind: None,
        item_input_hash: Some(format!("test-input-hash-{id}")),
        completion_evidence: Vec::new(),
    }
}

/// A live run accepted TDL-001 with `tests 2/4`: two `archon trading data`
/// commands exited 1 on a real registry checksum defect, and the verdict stayed
/// `accepted`. Neither existing guard applied — the commands were present, so
/// not "commandless", and they produced output, so not "zero-matched". Only the
/// absence of a rule asking "did a test fail?" let it through.
#[test]
fn accepted_verification_with_a_failing_test_command_is_demoted() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "audit completed; findings recorded".to_string(),
        ..WorkflowV2Result::default()
    };
    // A genuinely successful command, so the commandless backstop stays quiet.
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-trading data_store_tests".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "test result: ok. 12 passed; 0 failed".to_string(),
    });
    // The two that actually failed in the live run.
    for command in [
        "archon trading data status --target .",
        "archon trading data list --target . --json",
    ] {
        result.commands_run.push(crate::WorkflowV2CommandRecord {
            kind: crate::WorkflowV2CommandKind::Test,
            command: command.to_string(),
            status: crate::WorkflowV2CommandStatus::Failed,
            exit_code: Some(1),
            output_summary: "Failed closed on the live dataset checksum-chain mismatch".to_string(),
        });
    }
    result.data = serde_json::json!({ "canonical_task_ids": ["TASK-TDL-001"] });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "verify-TASK-TDL-001",
            WorkflowV2Status::Accepted,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    assert_eq!(result.data["outcomes"][0]["status"], "needs_review");
    let failed = &result.data["outcomes"][0]["result"]["data"]["failed_test_commands"];
    assert_eq!(
        failed.as_array().map(|items| items.len()),
        Some(2),
        "both failing commands must be named in the gap: {failed:#?}"
    );
}

/// The new rule must not fire on a clean run — every test succeeded, so the
/// acceptance stands. Without this, the guard could pass by demoting everything.
#[test]
fn accepted_verification_with_all_tests_passing_is_left_alone() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "all focused tests passed".to_string(),
        ..WorkflowV2Result::default()
    };
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-trading data_store_tests".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "test result: ok. 12 passed; 0 failed".to_string(),
    });
    result.task_coverage.push(crate::WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-001".to_string(),
        status: crate::WorkflowV2TaskCoverageStatus::Accepted,
        summary: "acceptance criteria proven by focused tests".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Test,
            "focused verification evidence",
        )],
    });
    result.data = serde_json::json!({ "canonical_task_ids": ["TASK-TDL-001"] });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "verify-TASK-TDL-001",
            WorkflowV2Status::Accepted,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
}

/// A failing command of a non-test kind must not demote: builds and inspections
/// fail for reasons that are not verification verdicts, and treating them as
/// such would block work the evidence does support.
#[test]
fn accepted_verification_with_a_failing_non_test_command_is_left_alone() {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "tests passed; an inspection probe failed".to_string(),
        ..WorkflowV2Result::default()
    };
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-trading data_store_tests".to_string(),
        status: crate::WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "test result: ok. 12 passed; 0 failed".to_string(),
    });
    result.commands_run.push(crate::WorkflowV2CommandRecord {
        kind: crate::WorkflowV2CommandKind::Inspect,
        command: "command -v some-optional-tool".to_string(),
        status: crate::WorkflowV2CommandStatus::Failed,
        exit_code: Some(1),
        output_summary: "not found".to_string(),
    });
    result.data = serde_json::json!({ "canonical_task_ids": ["TASK-TDL-001"] });

    let result = result_from_fanout_report(
        &fanout_call("verification-wave-1"),
        report(vec![outcome(
            "verify-TASK-TDL-001",
            WorkflowV2Status::Accepted,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
}
