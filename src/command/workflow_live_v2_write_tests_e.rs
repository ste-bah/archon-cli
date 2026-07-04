#[test]
fn verification_blocked_after_patch_is_contract_not_safety() {
    let error =
        "verification blocked after patch: agent output declares failed verification status";

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);

    let result = write_branch_validation_error_result("impl-task", None, error);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["failure_kind"], "contract");
}

#[test]
fn unsafe_path_still_classifies_as_safety() {
    let error = "write item 'impl-task' changed undeclared path 'src/other.rs'";

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Safety);

    let result = write_branch_validation_error_result("impl-task", None, error);

    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert_eq!(result.data["failure_kind"], "safety");
}

#[test]
fn oversized_candidate_file_is_review_data_not_terminal_stage_failure() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wfdc_size_policy_branch_failure.json"
    ))
    .expect("fixture");
    let error = fixture["old_error"].as_str().expect("old error");

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);

    let result =
        write_branch_validation_error_result(fixture["branch_id"].as_str().unwrap(), None, error);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["failure_kind"], "contract");
    assert!(result.residual_gaps[0].description.contains("exceeds max"));
}

#[test]
fn validation_error_preserves_source_task_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/write_validation_error_source_identity.json"
    ))
    .expect("fixture");
    let result = write_branch_validation_error_result(
        fixture["branch_id"].as_str().expect("branch id"),
        Some(&fixture["input"]),
        fixture["error"].as_str().expect("error"),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.data["canonical_task_ids"][0],
        fixture["input"]["item"]["canonical_task_ids"][0]
    );
    assert_eq!(result.data["branch_error_from_runtime"], true);
}
