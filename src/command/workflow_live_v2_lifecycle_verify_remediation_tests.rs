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
            ..Default::default()
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

include!("workflow_live_v2_lifecycle_verify_payload_tests.rs");

#[test]
fn mixed_triage_preserves_actionable_and_retry_routes() {
    let triage: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf3b9_verification_failure_triage_5_3.json"
    ))
    .expect("D25 fixture");

    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&triage);

    assert_eq!(routes.implementation_failures.len(), 1);
    assert_eq!(routes.retry_items.len(), 3);
    assert_eq!(
        routes.implementation_failures[0]["classification"],
        "actionable_implementation_failure"
    );
}

#[test]
fn d46_execution_retry_is_scheduled_even_when_write_inventory_is_empty() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/d46_orphaned_verification_retry.json"
    ))
    .expect("D46 fixture");
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-060".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-TDL-060.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    };
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let failed = vec![fixture["failed_outcome"].clone()];

    let retries = producer_retry_items(
        &contract,
        &fixture["triage"],
        workflow_live_v2_lifecycle_verify_routing::RetryProducer::Triage,
        &[fixture["plan_item"].clone()],
        &failed,
    )
    .expect("D46 execution retry must remain schedulable");
    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&fixture["triage"]);
    let plan = workflow_live_v2_lifecycle_verify_routing::triage_route_plan(&routes);

    assert_eq!(retries.len(), 1);
    assert_eq!(
        retries[0]["classification"],
        "retry_resolved_verification_execution_issue"
    );
    assert!(plan.run_retries);
    assert!(plan.try_supersede);
    assert!(!plan.run_write_remediation);
    assert_eq!(
        workflow_live_v2_lifecycle_verify_routing::remediation_inventory_route(&plan, false),
        workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::NotNeeded
    );
}

include!("workflow_live_v2_lifecycle_verify_retriage_tests.rs");

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

#[test]
fn d22_verification_remediation_source_items_satisfy_graph_contract() {
    let item: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf485_verification_remediation_source_item.json"
    ))
    .expect("D22 fixture");
    let source_items =
        workflow_live_v2_lifecycle_verify_merge::verification_remediation_source_items(
            &serde_json::json!({ "items": [item] }),
        );

    assert_eq!(
        source_items[0]["verification_requirements"],
        source_items[0]["focused_verification"]
    );
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-050".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-TDL-050.md".to_string(),
                dependency_ids: vec!["TASK-TDL-030".to_string()],
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-030".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-TDL-030.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
        ],
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-5-verification-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "source_data": source_items }),
        depends_on: Vec::new(),
    };
    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);
    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
}

#[test]
fn d23_retry_merge_preserves_unretried_failures() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wf485_verification_retry_merge.json"))
            .expect("D23 fixture");
    let retry_items = support::array(fixture.get("retry_items"));
    let merged = workflow_live_v2_lifecycle_verify_merge::merge_retry_outcomes(
        &fixture["initial"],
        fixture["retry_result"].clone(),
        &retry_items,
    );
    let outcomes = support::outcomes_of(&merged);

    assert_eq!(outcomes.len(), 9);
    assert_eq!(merged["status"], "needs_review");
    assert_eq!(
        merged["summary"],
        "verification retry merged 9 outcomes with 2 unresolved"
    );
    assert!(outcomes.iter().any(|outcome| {
        outcome["item_id"] == "verify-040-artifact" && outcome["status"] == "failed"
    }));
    assert!(
        !outcomes
            .iter()
            .any(|outcome| outcome["item_id"] == "verify-050-artifact")
    );
    assert!(outcomes.iter().any(|outcome| {
        outcome["source_item_id"] == "verify-050-artifact" && outcome["status"] == "accepted"
    }));
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

fn corrected_retry_item(outcome: &serde_json::Value) -> serde_json::Value {
    let item_id = outcome["item_id"].as_str().expect("item id");
    let gap = &outcome["result"]["residual_gaps"][0];
    serde_json::json!({
        "item_id": item_id,
        "source_item_id": item_id,
        "canonical_task_ids": ["TASK-TDL-010"],
        "classification": "retryable_verification_shape_issue",
        "source_residual_gap_ids": [gap["id"].clone()],
        "failed_predicate": gap["description"].clone(),
        "focused_verification": ["cargo test trading_data_validation_and_provider_commands_parse"],
        "expected_evidence": ["The corrected exact test name runs and passes."]
    })
}
