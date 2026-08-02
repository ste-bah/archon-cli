use super::*;

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
fn coordinator_undeclared_patch_wording_is_branch_scoped_safety_data() {
    let error = "patch writes undeclared path 'src/other.rs'";

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Safety);
}

#[test]
fn compound_repair_failure_uses_root_rejection_kind() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/d26_compound_repair_failure.json"))
            .expect("fixture");
    let error = fixture["error"].as_str().expect("error");

    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);
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

#[test]
fn post_parse_worktree_rejection_persists_the_result_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let result = WorkflowV2Result::accepted("agent accepted without enough evidence");

    persist_rejected_worktree_result(
        &store,
        "implementation-branch",
        "validation",
        &result,
        "implementation artifact declares accepted status without required evidence fields",
    );

    let body = std::fs::read_to_string(store.rejected_output_path("implementation-branch"))
        .expect("rejected output log");
    let log: serde_json::Value = serde_json::from_str(&body).expect("rejected output JSON");
    assert_eq!(log["rejections"][0]["attempt"], "validation");
    assert!(
        log["rejections"][0]["raw_body"]
            .as_str()
            .is_some_and(|body| body.contains("agent accepted without enough evidence"))
    );
}
