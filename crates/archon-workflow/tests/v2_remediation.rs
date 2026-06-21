use archon_workflow::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2ConvergenceController, WorkflowV2ConvergenceError, WorkflowV2ConvergenceStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2VerificationKind,
    WorkflowV2VerificationOutcome, WorkflowV2VerificationStatus,
};

#[test]
fn failed_test_triggers_remediation() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let decision = controller
        .evaluate(0, &[focused_test(WorkflowV2VerificationStatus::Failed)])
        .expect("decision");

    assert_eq!(decision.status, WorkflowV2ConvergenceStatus::Remediate);
    assert!(decision.requires_reverification);
    assert_eq!(decision.remediation_items.len(), 1);
    assert_eq!(
        decision.remediation_items[0].commands_to_rerun,
        vec!["cargo test focused"]
    );
    decision.result.validate().unwrap();
}

#[test]
fn failed_review_triggers_remediation() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let decision = controller
        .evaluate(
            0,
            &[
                focused_test(WorkflowV2VerificationStatus::Passed),
                review(WorkflowV2VerificationStatus::Failed),
            ],
        )
        .expect("decision");

    assert_eq!(decision.status, WorkflowV2ConvergenceStatus::Remediate);
    assert!(decision.requires_reverification);
    assert_eq!(
        decision.remediation_items[0].source_kind,
        WorkflowV2VerificationKind::AdversarialReview
    );
    decision.result.validate().unwrap();
}

#[test]
fn remediation_success_reruns_verification_and_accepts() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let first = controller
        .evaluate(0, &[focused_test(WorkflowV2VerificationStatus::Failed)])
        .expect("first");
    assert!(first.requires_reverification);

    let second = controller
        .evaluate(
            1,
            &[
                focused_test(WorkflowV2VerificationStatus::Passed),
                review(WorkflowV2VerificationStatus::Passed),
            ],
        )
        .expect("second");

    assert_eq!(second.status, WorkflowV2ConvergenceStatus::Accepted);
    assert!(!second.requires_reverification);
    assert!(second.remediation_items.is_empty());
    second.result.validate().unwrap();
}

#[test]
fn max_iterations_stop_with_blocking_evidence() {
    let controller = WorkflowV2ConvergenceController::new(1);
    let decision = controller
        .evaluate(1, &[focused_test(WorkflowV2VerificationStatus::Failed)])
        .expect("blocked");

    assert_eq!(decision.status, WorkflowV2ConvergenceStatus::Blocked);
    assert!(!decision.requires_reverification);
    assert!(
        decision
            .result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "max_iterations")
    );
    decision.result.validate().unwrap();
}

#[test]
fn blocked_requires_concrete_external_blocker() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let err = controller
        .evaluate(0, &[focused_test(WorkflowV2VerificationStatus::Blocked)])
        .expect_err("missing blocker");

    assert_eq!(
        err,
        WorkflowV2ConvergenceError::MissingExternalBlocker("T001".to_string())
    );
}

#[test]
fn concrete_external_blocker_blocks_with_evidence() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let mut outcome = focused_test(WorkflowV2VerificationStatus::Blocked);
    outcome.external_blocker = Some("required external service is unavailable".to_string());

    let decision = controller.evaluate(0, &[outcome]).expect("blocked");

    assert_eq!(decision.status, WorkflowV2ConvergenceStatus::Blocked);
    assert!(
        decision
            .result
            .evidence
            .iter()
            .any(|evidence| evidence.kind == WorkflowV2EvidenceKind::Blocker)
    );
    decision.result.validate().unwrap();
}

#[test]
fn test_listing_does_not_count_as_execution() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let mut outcome = focused_test(WorkflowV2VerificationStatus::Passed);
    outcome.command = Some(test_cmd(
        "cargo test focused -- --list=json",
        WorkflowV2CommandStatus::Succeeded,
    ));

    let err = controller.evaluate(0, &[outcome]).expect_err("listing");

    assert!(matches!(
        err,
        WorkflowV2ConvergenceError::ListingCommandIsNotExecution { task_id, .. }
            if task_id == "T001"
    ));
}

#[test]
fn acceptance_requires_focused_test_and_review_outcomes() {
    let controller = WorkflowV2ConvergenceController::new(3);

    let test_only = controller
        .evaluate(0, &[focused_test(WorkflowV2VerificationStatus::Passed)])
        .expect_err("missing review");
    assert_eq!(
        test_only,
        WorkflowV2ConvergenceError::MissingReviewOutcomeForAcceptance
    );

    let review_only = controller
        .evaluate(0, &[review(WorkflowV2VerificationStatus::Passed)])
        .expect_err("missing focused test");
    assert_eq!(
        review_only,
        WorkflowV2ConvergenceError::MissingFocusedTestOutcomeForAcceptance
    );
}

#[test]
fn review_without_evidence_is_rejected() {
    let controller = WorkflowV2ConvergenceController::new(3);
    let mut outcome = review(WorkflowV2VerificationStatus::Passed);
    outcome.evidence.clear();

    let err = controller
        .evaluate(0, &[outcome])
        .expect_err("review evidence");

    assert_eq!(
        err,
        WorkflowV2ConvergenceError::MissingReviewEvidence("T001".to_string())
    );
}

fn focused_test(status: WorkflowV2VerificationStatus) -> WorkflowV2VerificationOutcome {
    let command_status = match status {
        WorkflowV2VerificationStatus::Passed => WorkflowV2CommandStatus::Succeeded,
        WorkflowV2VerificationStatus::Failed | WorkflowV2VerificationStatus::Blocked => {
            WorkflowV2CommandStatus::Failed
        }
    };
    WorkflowV2VerificationOutcome {
        kind: WorkflowV2VerificationKind::FocusedTest,
        task_id: "T001".to_string(),
        status,
        summary: match status {
            WorkflowV2VerificationStatus::Passed => "focused tests passed",
            WorkflowV2VerificationStatus::Failed => "focused tests failed",
            WorkflowV2VerificationStatus::Blocked => "focused tests blocked",
        }
        .to_string(),
        command: Some(test_cmd("cargo test focused", command_status)),
        evidence: Vec::new(),
        external_blocker: None,
    }
}

fn review(status: WorkflowV2VerificationStatus) -> WorkflowV2VerificationOutcome {
    WorkflowV2VerificationOutcome {
        kind: WorkflowV2VerificationKind::AdversarialReview,
        task_id: "T001".to_string(),
        status,
        summary: match status {
            WorkflowV2VerificationStatus::Passed => "adversarial review passed",
            WorkflowV2VerificationStatus::Failed => "adversarial review found a gap",
            WorkflowV2VerificationStatus::Blocked => "adversarial review blocked",
        }
        .to_string(),
        command: None,
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "reviewed implementation against acceptance criteria",
        )],
        external_blocker: None,
    }
}

fn test_cmd(command: &str, status: WorkflowV2CommandStatus) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.to_string(),
        status,
        exit_code: None,
        output_summary: "focused command output".to_string(),
    }
}
