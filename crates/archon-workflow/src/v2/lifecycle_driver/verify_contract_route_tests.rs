//! A missing product is repaired without rewriting its declared contract.
use crate::v2::deliverable_contract::ContractRoots;
use crate::v2::{WorkflowV2BranchOutcome, WorkflowV2Result, WorkflowV2Status};
use serde_json::json;

fn accepted() -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: "floor".into(),
        role: "verifier".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("verified")),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

#[tokio::test]
async fn typed_floor_fixed_contract_accepts_the_repaired_product() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.txt"), "evidence").unwrap();
    let product = directory.path().join("product.sh");
    let command = format!(
        "{} {}",
        crate::acceptance_scratch::shell_arg(archon_shell::resolve_posix_shell()),
        crate::acceptance_scratch::shell_arg(&product)
    );
    let contracts = std::collections::BTreeMap::from([(
        "floor".into(),
        (
            ContractRoots::project_only(directory.path().to_string_lossy()),
            vec![
                json!({"kind":"evidence","artifact_path":"out.txt","typed_verifier_command":command}),
            ],
        ),
    )]);
    let mut failed = [accepted()];
    crate::v2::verification::enforce_declared_contracts(&mut failed, &contracts).await;
    assert_eq!(failed[0].status, WorkflowV2Status::NeedsReview);
    assert!(super::verify_contract_route::failed_with_residual_gaps(
        &serde_json::to_value(&failed[0]).unwrap()
    ));
    std::fs::write(&product, "exit 0\n").unwrap();
    let mut repaired = [accepted()];
    crate::v2::verification::enforce_declared_contracts(&mut repaired, &contracts).await;
    assert_eq!(repaired[0].status, WorkflowV2Status::Accepted);
    // Site-missing behavior runs with the unchanged classifier in the binary's
    // missing_typed_program_host_wave_records_unverified_and_keeps_product_failures.
}
