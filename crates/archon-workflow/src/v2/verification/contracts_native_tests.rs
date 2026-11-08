use super::*;
use crate::v2::WorkflowV2Result;

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
async fn typed_command_zero_nonzero_and_json_failure_follow_passing_predicates() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.json"), "{}").unwrap();
    for (command, expected) in [
        ("exit 0", WorkflowV2Status::Accepted),
        ("exit 3", WorkflowV2Status::NeedsReview),
        ("(exit 3)", WorkflowV2Status::NeedsReview),
        (
            r#"printf '%s\n' '{"status":"failed","failures":["typed failure"]}'"#,
            WorkflowV2Status::NeedsReview,
        ),
    ] {
        let mut outcomes = [accepted()];
        let contracts = std::collections::BTreeMap::from([(
            "floor".into(),
            (
                ContractRoots::project_only(directory.path().to_string_lossy()),
                vec![
                    serde_json::json!({"kind":"evidence","artifact_path":"out.json","typed_verifier_command":command}),
                ],
            ),
        )]);
        enforce_declared_contracts(&mut outcomes, &contracts).await;
        assert_eq!(outcomes[0].status, expected, "{command}: {:?}", outcomes[0]);
        assert_eq!(
            outcomes[0].result.as_ref().unwrap().data["declared_contract_evaluator"],
            "shared_declarative_floor_and_typed_command"
        );
    }
}

#[tokio::test]
async fn failed_predicates_never_run_the_typed_command() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("marker");
    let command = format!(
        "printf ran > {}",
        crate::acceptance_scratch::shell_arg(&marker)
    );
    let mut outcomes = [accepted()];
    let contracts = std::collections::BTreeMap::from([(
        "floor".into(),
        (
            ContractRoots::project_only(directory.path().to_string_lossy()),
            vec![
                serde_json::json!({"kind":"evidence","artifact_path":"missing.json","typed_verifier_command":command}),
            ],
        ),
    )]);
    enforce_declared_contracts(&mut outcomes, &contracts).await;
    assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
    assert!(!marker.exists());
    assert_eq!(
        outcomes[0].result.as_ref().unwrap().data["declared_contract_evaluator"],
        "shared_declarative_floor"
    );
}

#[tokio::test]
async fn malformed_contract_is_a_failed_floor() {
    let directory = tempfile::tempdir().unwrap();
    for raw in [
        serde_json::json!({"kind":"evidence","artifact_path":"out.json","unknown":true}),
        serde_json::json!({"kind":"evidence","artifact_path":"out.json","unknown":""}),
        serde_json::json!({"artifact_path":"out.json"}),
        serde_json::json!({"kind":"evidence","artifact_path":"out.json","min_instances":"1"}),
    ] {
        let mut outcomes = [accepted()];
        let contracts = std::collections::BTreeMap::from([(
            "floor".into(),
            (
                ContractRoots::project_only(directory.path().to_string_lossy()),
                vec![raw],
            ),
        )]);
        enforce_declared_contracts(&mut outcomes, &contracts).await;
        assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
        let findings = &outcomes[0].result.as_ref().unwrap().data["declared_contract_findings"];
        assert!(
            findings[0]
                .as_str()
                .unwrap()
                .starts_with("declared deliverable contract does not parse:")
        );
    }
}

#[tokio::test]
async fn product_and_command_failures_stay_in_the_implementer_route() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.json"), "{}").unwrap();
    for command in ["./absent-product", "exit 126", "exit 127", "if then"] {
        let started = std::time::Instant::now();
        let mut outcomes = [accepted()];
        let contracts = std::collections::BTreeMap::from([(
            "floor".into(),
            (
                ContractRoots::project_only(directory.path().to_string_lossy()),
                vec![
                    serde_json::json!({"kind":"evidence","artifact_path":"out.json","typed_verifier_command":command}),
                ],
            ),
        )]);
        enforce_declared_contracts(&mut outcomes, &contracts).await;
        assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
        let result = outcomes[0].result.as_ref().unwrap();
        assert_eq!(
            result.data["verification_failure_class"],
            "declared_contract_violation"
        );
        assert!(
            crate::v2::lifecycle_driver::verify_contract_route::failed_with_residual_gaps(
                &serde_json::to_value(&outcomes[0]).unwrap()
            )
        );
        // The fixed contract stays unchanged. Only the missing implementation
        // can be supplied here; unavailable-site classification is tested on
        // the host path with its real existing classifier.
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }
}
