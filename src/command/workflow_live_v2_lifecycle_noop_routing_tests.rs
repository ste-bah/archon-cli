use super::*;
use crate::command::workflow_live::workflow_live_v2::workflow_live_v2_script::{
    item_has_write_ownership, preserve_host_pinned_implementation,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn contract() -> (WorkflowV2TaskUniverse, Value) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-001".to_string(),
            source_path: "tasks/TASK-TDL-001.md".to_string(),
            title: Some("Data Lake Gap Audit".to_string()),
            acceptance_criteria: vec![
                "Gap report maps current code and missing implementation to every normative requirement."
                    .to_string(),
                "Existing registry behavior is documented honestly.".to_string(),
                "No storage-root change is proposed.".to_string(),
            ],
            artifact_requirements: vec!["project/artifacts/gap-audit-current.json".to_string()],
            ..Default::default()
        }],
    };
    let fixture = serde_json::from_str(archon_test_support::fixtures::D60_REFUTED_NOOP_ROUTING)
        .expect("D60 fixture");
    (universe, fixture)
}

#[test]
fn d61_host_pins_criteria_and_refutes_both_inconsistent_canary_shapes() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let canary: Value =
        serde_json::from_str(archon_test_support::fixtures::D61_NOOP_ACCEPTANCE_CONSISTENCY)
            .expect("D61 fixture");
    let ready = pin_noop_acceptance_criteria(
        &contract,
        &support::array(fixture["inventory"].get("items")),
    );

    assert_eq!(
        ready[0]["acceptance_criteria"],
        serde_json::json!([
            "Gap report maps current code and missing implementation to every normative requirement.",
            "Existing registry behavior is documented honestly.",
            "No storage-root change is proposed."
        ])
    );
    for outcome in ["previously_refuted", "previously_accepted"] {
        let enforced =
            enforce_noop_acceptance_criteria(&contract, &ready, &[canary[outcome].clone()]);
        assert_eq!(enforced[0]["status"], "needs_review", "{outcome}");
        if outcome == "previously_accepted" {
            assert!(
                support::array(enforced[0].get("residual_gaps"))
                    .iter()
                    .any(|gap| gap["id"] == "noop-acceptance-criteria-unsatisfied"),
                "{outcome}"
            );
        }
    }
}

#[test]
fn discovery_merge_cannot_flip_host_demoted_work_back_to_noop() {
    let (universe, _) = contract();
    let lifecycle_contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let inventory = serde_json::json!({"items": [{
        "item_id": "refuted-noop",
        "work_type": "verified_noop",
        "canonical_task_ids": ["TASK-TDL-001"],
        "dependency_ids": [],
        "acceptance_criteria": [
            "Gap report maps current code and missing implementation to every normative requirement.",
            "Existing registry behavior is documented honestly.",
            "No storage-root change is proposed."
        ],
        "artifact_requirements": ["project/artifacts/gap-audit-current.json"]
    }]});
    let demoted = std::collections::BTreeSet::from(["TASK-TDL-001".to_string()]);

    let pinned = preserve_host_pinned_implementation(&lifecycle_contract, &inventory, &demoted);
    let item = &pinned["items"][0];
    assert_eq!(item["work_type"], "implementation");
    assert!(item_has_write_ownership(item));
}

#[test]
fn noop_credit_requires_one_exact_evidenced_result_per_criterion() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let ready = pin_noop_acceptance_criteria(
        &contract,
        &support::array(fixture["inventory"].get("items")),
    );
    let results = universe.tasks[0]
        .acceptance_criteria
        .iter()
        .map(|criterion| {
            serde_json::json!({
                "task_id": "TASK-TDL-001",
                "criterion": criterion,
                "status": "passed",
                "evidence_refs": ["project/artifacts/gap-audit-current.json"],
            })
        })
        .collect::<Vec<_>>();
    let outcome = serde_json::json!({
        "status": "noop",
        "canonical_task_ids": ["TASK-TDL-001"],
        "evidence": ["project/artifacts/gap-audit-current.json"],
        "acceptance_criteria_results": results,
    });

    let enforced = enforce_noop_acceptance_criteria(&contract, &ready, &[outcome]);

    assert_eq!(enforced[0]["status"], "noop");
}

#[test]
fn refuted_noop_after_bounded_repairs_routes_to_implementation() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    assert_eq!(
        fixture["noop_repair_rounds"]
            .as_array()
            .expect("repair rounds")
            .len(),
        3
    );
    let mut ready = support::array(fixture["inventory"].get("items"));
    ready[0]["target_files"] = serde_json::json!([]);
    ready[0]["artifact_requirements"] = serde_json::json!([]);
    let failed = support::array(fixture.get("failed_noop_outcomes"));
    let mut reclassified = BTreeSet::new();

    let route = route_refuted_noops(
        &contract,
        &ready,
        &BTreeSet::new(),
        &failed,
        &BTreeSet::new(),
        &mut reclassified,
    );

    let NoopProofExhaustionRoute::ScheduleImplementation(items) = route else {
        panic!("refuted noop must schedule implementation");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["work_type"], "implementation");
    assert_eq!(items[0]["canonical_task_ids"][0], "TASK-TDL-001");
    assert_eq!(items[0]["noop_reclassification"]["count"], 1);
    assert_eq!(items[0]["target_files"], serde_json::json!([]));
    assert_eq!(
        items[0]["artifact_requirements"],
        serde_json::json!(["project/artifacts/gap-audit-current.json"])
    );
}

#[test]
fn refuted_noop_waits_for_dependencies_before_implementation() {
    let (mut universe, fixture) = contract();
    universe.tasks[0].dependency_ids = vec!["TASK-TDL-000".to_string()];
    universe.tasks.insert(
        0,
        WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-000".to_string(),
            source_path: "tasks/TASK-TDL-000.md".to_string(),
            acceptance_criteria: vec!["Prerequisite is complete.".to_string()],
            ..Default::default()
        },
    );
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let mut ready = support::array(fixture["inventory"].get("items"));
    ready[0]["dependency_ids"] = serde_json::json!(["TASK-TDL-000"]);
    let failed = support::array(fixture.get("failed_noop_outcomes"));

    assert_eq!(
        route_refuted_noops(
            &contract,
            &ready,
            &BTreeSet::new(),
            &failed,
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        ),
        NoopProofExhaustionRoute::Block
    );

    let completed = BTreeSet::from(["TASK-TDL-000".to_string()]);
    assert!(matches!(
        route_refuted_noops(
            &contract,
            &ready,
            &BTreeSet::new(),
            &failed,
            &completed,
            &mut BTreeSet::new(),
        ),
        NoopProofExhaustionRoute::ScheduleImplementation(_)
    ));
}

#[test]
fn noop_reclassification_is_bounded_once_per_task() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let ready = support::array(fixture["inventory"].get("items"));
    let failed = support::array(fixture.get("failed_noop_outcomes"));
    let mut reclassified = BTreeSet::new();

    assert!(matches!(
        route_refuted_noops(
            &contract,
            &ready,
            &BTreeSet::new(),
            &failed,
            &BTreeSet::new(),
            &mut reclassified,
        ),
        NoopProofExhaustionRoute::ScheduleImplementation(_)
    ));
    assert_eq!(
        route_refuted_noops(
            &contract,
            &ready,
            &BTreeSet::new(),
            &failed,
            &BTreeSet::new(),
            &mut reclassified,
        ),
        NoopProofExhaustionRoute::Block
    );
}

#[test]
fn transport_failure_does_not_reclassify_noop_as_implementation() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let ready = support::array(fixture["inventory"].get("items"));
    let failed = vec![serde_json::json!({
        "status": "failed",
        "data": {
            "failure_class": "transport_infrastructure",
            "canonical_task_ids": ["TASK-TDL-001"]
        }
    })];

    assert_eq!(
        route_refuted_noops(
            &contract,
            &ready,
            &BTreeSet::new(),
            &failed,
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        ),
        NoopProofExhaustionRoute::Block
    );
}

#[test]
fn mixed_noop_failures_reclassify_only_the_task_with_a_semantic_gap() {
    let (mut universe, fixture) = contract();
    universe.tasks.push(WorkflowV2TaskUniverseTask {
        canonical_task_id: "TASK-TDL-002".to_string(),
        source_path: "tasks/TASK-TDL-002.md".to_string(),
        artifact_requirements: vec!["project/artifacts/second.json".to_string()],
        ..Default::default()
    });
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let mut ready = support::array(fixture["inventory"].get("items"));
    let mut second = ready[0].clone();
    second["item_id"] = Value::String("noop-second".to_string());
    second["id"] = Value::String("noop-second".to_string());
    second["canonical_task_ids"] = serde_json::json!(["TASK-TDL-002"]);
    second["artifact_requirements"] = serde_json::json!(["project/artifacts/second.json"]);
    ready.push(second);
    let failed = vec![serde_json::json!({
        "status": "needs_review",
        "canonical_task_ids": ["TASK-TDL-001"],
        "residual_gaps": [{"id": "gap-one", "description": "first task is incomplete"}]
    })];

    let NoopProofExhaustionRoute::ScheduleImplementation(items) = route_refuted_noops(
        &contract,
        &ready,
        &BTreeSet::new(),
        &failed,
        &BTreeSet::new(),
        &mut BTreeSet::new(),
    ) else {
        panic!("semantic gap must schedule implementation");
    };

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["canonical_task_ids"][0], "TASK-TDL-001");
}

#[test]
fn inventory_task_coverage_contradiction_skips_noop_proof() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let mut inventory = fixture["inventory"].clone();
    inventory["task_coverage"] = serde_json::json!([{
        "task_id": "TASK-TDL-001",
        "status": "partial"
    }]);

    let (inventory, ids) = reclassify_inventory_contradicted_noops(&contract, &inventory);
    let items = support::array(inventory.get("items"));

    assert!(ids.contains("TASK-TDL-001"));
    assert_eq!(items[0]["work_type"], "implementation");
}

#[test]
fn inventory_artifact_gap_contradiction_skips_noop_proof() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let mut inventory = fixture["inventory"].clone();
    inventory["residual_gaps"] = serde_json::json!([{
        "id": "gap-current-audit",
        "description": "project/artifacts/gap-audit-current.json is incomplete"
    }]);

    let (inventory, ids) = reclassify_inventory_contradicted_noops(&contract, &inventory);
    let items = support::array(inventory.get("items"));

    assert!(ids.contains("TASK-TDL-001"));
    assert_eq!(items[0]["work_type"], "implementation");
}

#[test]
fn canary_inventory_family_gap_contradiction_skips_noop_proof() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };

    let (inventory, ids) =
        reclassify_inventory_contradicted_noops(&contract, &fixture["inventory"]);
    let items = support::array(inventory.get("items"));

    assert!(ids.contains("TASK-TDL-001"));
    assert_eq!(items[0]["work_type"], "implementation");
}

#[test]
fn unrelated_inventory_gap_does_not_demote_noop_claim() {
    let (universe, fixture) = contract();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let mut inventory = fixture["inventory"].clone();
    inventory["residual_gaps"] = serde_json::json!([{
        "id": "GAP-OTHER-NOTE",
        "description": "An unrelated operator note is missing."
    }]);

    let (inventory, ids) = reclassify_inventory_contradicted_noops(&contract, &inventory);
    let items = support::array(inventory.get("items"));

    assert!(ids.is_empty());
    assert_eq!(items[0]["work_type"], "verified_noop");
}
