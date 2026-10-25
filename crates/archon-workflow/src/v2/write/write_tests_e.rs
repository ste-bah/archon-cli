use super::*;
use std::sync::Mutex;

use crate::{WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest};

#[test]
fn verification_blocked_after_patch_is_contract_not_safety() {
    let error =
        "verification blocked after patch: agent output declares failed verification status";

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);

    let result = write_branch_validation_error_result("TASK-X-001", None, error);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["failure_kind"], "contract");
}

#[tokio::test]
async fn criterion_text_does_not_change_the_needs_review_classification() {
    struct ExhaustingClient(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl WorkflowV2AgentClient for ExhaustingClient {
        async fn run_agent(&self, _: String) -> Result<String, WorkflowV2AgentError> {
            Ok(self.0.lock().unwrap().remove(0))
        }

        async fn continue_agent_request(
            &self,
            _: &WorkflowV2AgentRequest,
            _: String,
        ) -> Result<String, WorkflowV2AgentError> {
            self.run_agent(String::new()).await
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("owned.txt");
    std::fs::write(&path, "changed").unwrap();
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "call-x-001".into(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".into(),
        task: "Complete the declared validation task".into(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": {"canonical_task_ids": ["TASK-X-001"]},
            "task_universe": {"tasks": [{
                "canonical_task_id": "TASK-X-001",
                "acceptance_criteria": ["read-only verification timed out"]
            }]}
        }),
        repository_root: Some(temp.path().display().to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["owned.txt".into()],
        target_ownership_scopes: Vec::new(),
    };
    let mut malformed = WorkflowV2Result::accepted("completed");
    malformed.evidence.push(crate::WorkflowV2Evidence::new(
        crate::WorkflowV2EvidenceKind::Implementation,
        "updated the declared validation file",
    ));
    malformed
        .files_changed
        .push(WorkflowV2FileRecord::new(path.display().to_string()));
    malformed.data = serde_json::json!({"criterion_results": [{
        "task_id": "TASK-X-001", "criterion_index": 1,
        "status": "invalid", "evidence": "the shape is invalid"
    }]});
    let output = serde_json::to_string(&malformed).unwrap();
    let client = ExhaustingClient(Mutex::new(vec![output.clone(), output]));
    let error = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .unwrap_err();
    let error_text = error.to_string();
    assert!(error_text.contains("read-only"), "{error_text}");
    assert!(error_text.contains("timed out"), "{error_text}");
    let classification = write_branch_error_kind(&error_text);
    assert_eq!(classification, BranchFailureKind::Contract, "{error_text}");
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
        serde_json::from_str(archon_test_support::fixtures::D26_COMPOUND_REPAIR_FAILURE)
            .expect("fixture");
    let error = fixture["error"].as_str().expect("error");

    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);
}

#[test]
fn oversized_candidate_file_is_review_data_not_terminal_stage_failure() {
    let fixture: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::WFDC_SIZE_POLICY_BRANCH_FAILURE)
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
    let fixture: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::WRITE_VALIDATION_ERROR_SOURCE_IDENTITY)
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
