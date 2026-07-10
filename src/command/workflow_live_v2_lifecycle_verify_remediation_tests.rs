use super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask;
use super::*;

fn contract_fixture() -> (WorkflowV2TaskUniverse, serde_json::Value) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-010".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-TDL-010.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
        }],
    };
    let plan_item = serde_json::json!({
        "item_id": "verify-plan",
        "canonical_task_ids": ["TASK-TDL-010"],
        "focused_verification": ["check"],
        "expected_evidence": ["evidence"],
    });
    (universe, plan_item)
}

#[test]
fn fabel_triage_retry_items_keep_only_required_reruns() {
    let (universe, plan_item) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let triage: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wffed_verification_failure_triage_1_2.json"
    ))
    .expect("fixture json");

    let source_outcomes = vec![failed_outcome_with_gap(
        "verification-wave-1-1-VERIFY-TDL-010-003-project-registry-artifact-contract-check-7",
        "gap-healthy-dataset-required-artifact-path-fields",
    )];
    let retry_items = triage_retry_items(&contract, &triage, &[plan_item], &source_outcomes);

    assert!(
        retry_items.is_none(),
        "legacy triage fixture retains unrelated contract defects"
    );
}

#[test]
fn fabel_supersede_accepts_shape_failure_with_sibling_evidence() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            accepted_outcome("accepted-sibling"),
            failed_outcome_with_gap("failed-shape", "shape-gap")
        ]
    });
    let triage = serde_json::json!({
        "status": "accepted",
        "data": {
            "implementation_failures": [],
            "terminal_blockers": [],
            "retry_items": [{
                "item_id": "failed-shape",
                "classification": "retryable_verification_shape_issue",
                "source_residual_gap_ids": ["shape-gap"],
                "failed_predicate": "failed invariant"
            }]
        }
    });

    let supersede = workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
        &contract,
        &verification,
        &triage,
        "verification-failure-triage",
    )
    .expect("supersede");

    assert_eq!(supersede.verification["status"], "accepted");
    assert_eq!(
        supersede.record["superseded"][0]["failed_outcome_id"],
        "failed-shape"
    );
}

#[test]
fn supersede_rejects_sibling_evidence_for_a_different_invariant() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            accepted_outcome("accepted-sibling"),
            failed_outcome_with_gap("failed-shape", "provider-env-mismatch")
        ]
    });
    let triage = serde_json::json!({
        "status": "accepted",
        "data": {
            "implementation_failures": [],
            "terminal_blockers": [],
            "retry_items": [{
                "item_id": "failed-shape",
                "classification": "retryable_verification_shape_issue",
                "source_residual_gap_ids": ["evidence-shape-gap"],
                "failed_predicate": "evidence envelope must be valid"
            }]
        }
    });

    assert!(
        workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
            &contract,
            &verification,
            &triage,
            "verification-failure-triage",
        )
        .is_none()
    );
}

#[test]
fn retry_inventory_stamps_a_dropped_source_gap() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture");

    let inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["invalid_retry_plan"],
        &fixture["initial_verification"],
    );

    assert!(support::array(inventory.get("unresolved_issues")).is_empty());
    assert_eq!(
        inventory["items"][0]["source_residual_gap_ids"],
        fixture["valid_retry_plan"]["items"][0]["source_residual_gap_ids"]
    );
}

#[test]
fn canary_shape_repair_rounds_use_stable_host_gap_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf6dd_verification_retry_invariant_failure.json"
    ))
    .expect("fixture JSON");
    for key in ["round_1_inventory", "round_3_inventory"] {
        let checked = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
            &fixture[key],
            &fixture["verification"],
        );
        assert!(support::array(checked.get("unresolved_issues")).is_empty());
        assert_eq!(
            checked["items"][0]["source_residual_gap_ids"],
            serde_json::json!(["manifest-required-evidence-missing"])
        );
        assert_eq!(
            checked["items"][0]["failed_predicate"],
            "Manifest must include registry and focused command evidence."
        );
    }
}

#[test]
fn retry_inventory_without_matching_failure_is_rejected() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf6dd_verification_retry_invariant_failure.json"
    ))
    .expect("fixture JSON");
    let checked = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["unmatched_inventory"],
        &fixture["verification"],
    );

    assert!(
        support::array(checked.get("unresolved_issues"))
            .iter()
            .any(|issue| issue["field"] == "source_item_id")
    );
}

#[test]
fn retry_inventory_accepts_the_exact_source_gap_and_predicate() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture");

    let inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["valid_retry_plan"],
        &fixture["initial_verification"],
    );

    assert!(support::array(inventory.get("unresolved_issues")).is_empty());
}

#[test]
fn fabel_shape_repair_drops_already_accepted_source_outcomes() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "outcomes": [accepted_outcome("accepted-source"), failed_outcome("failed-source")]
    });
    let inventory = serde_json::json!({
        "items": [
            retry_item("retry-accepted", "accepted-source"),
            retry_item("retry-failed", "failed-source")
        ]
    });

    let scoped = scope_repair_inventory_to_failed_outcomes(&contract, &inventory, &verification);
    let items = support::array(scoped.get("items"));

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["item_id"], "retry-failed");
}

fn accepted_outcome(id: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "status": "accepted",
        "canonical_task_ids": ["TASK-TDL-010"],
        "evidence": [{ "summary": "shape-gap resolved: evidence envelope must be valid" }],
        "resolved_residual_gap_ids": ["shape-gap"]
    })
}

fn failed_outcome(id: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "status": "failed",
        "canonical_task_ids": ["TASK-TDL-010"],
        "result": { "data": { "verification_failure_class": "retryable_verification_issue" } }
    })
}

fn failed_outcome_with_gap(id: &str, gap_id: &str) -> serde_json::Value {
    let mut outcome = failed_outcome(id);
    outcome["result"]["residual_gaps"] = serde_json::json!([{
        "id": gap_id,
        "description": "failed invariant",
        "severity": "review"
    }]);
    outcome
}

fn retry_item(id: &str, source: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "source_outcome_item_ids": [source],
        "canonical_task_ids": ["TASK-TDL-010"],
        "focused_verification": ["retry check"],
        "expected_evidence": ["retry evidence"]
    })
}
