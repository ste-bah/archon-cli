use super::manifest_scope_verification_result;
use crate::v2::WorkflowV2Status;

#[test]
fn shared_tree_noise_cannot_fail_manifest_grounded_scope() {
    let input = serde_json::json!({
        "item_id": "post-remediation-verify-owned-diff-scope",
        "focused_verification": "pwd && git status --short && git diff --stat",
        "expected_evidence": ["Diff is limited to owned source files."],
        "canonical_task_ids": ["TASK-001"],
        "write_coordination_scope": {
            "declared_target_files": ["src/owned.rs", "src/unchanged.rs"],
            "changed_files": ["src/owned.rs"],
            "created_files": [],
            "deleted_files": []
        }
    });

    let result = manifest_scope_verification_result(&input).expect("scope result");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.commands_run[0].command,
        "host:validate_write_coordination_scope"
    );
}

#[test]
fn undeclared_manifest_change_still_fails_closed() {
    let input = serde_json::json!({
        "focused_verification": "verify ownership diff",
        "canonical_task_ids": ["TASK-001"],
        "write_coordination_scope": {
            "declared_target_files": ["src/owned.rs"],
            "changed_files": ["src/unowned.rs"]
        }
    });

    let result = manifest_scope_verification_result(&input).expect("scope result");

    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert_eq!(
        result.residual_gaps[0].id,
        "write-coordination-scope-escape"
    );
}

#[test]
fn idempotent_noop_accepts_declared_files_with_no_observed_writes() {
    let input = serde_json::json!({
        "focused_verification": "verify ownership diff scope for noop",
        "canonical_task_ids": ["TASK-001"],
        "write_coordination_scope": {
            "declared_target_files": ["src/owned.rs", "src/also-owned.rs"],
            "changed_files": [],
            "created_files": [],
            "deleted_files": [],
            "status": {"status": "idempotent_noop"}
        }
    });

    let result = manifest_scope_verification_result(&input).expect("scope result");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.commands_run[0].exit_code, Some(0));
}
