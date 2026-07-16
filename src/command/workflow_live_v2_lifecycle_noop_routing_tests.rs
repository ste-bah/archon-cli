use super::super::super::super::workflow_live_task_universe::{
    WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use super::*;

fn contract() -> (WorkflowV2TaskUniverse, Value) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-001".to_string(),
            source_path: "tasks/TASK-TDL-001.md".to_string(),
            title: Some("Data Lake Gap Audit".to_string()),
            artifact_requirements: vec!["project/artifacts/gap-audit-current.json".to_string()],
            ..Default::default()
        }],
    };
    let fixture = serde_json::from_str(include_str!("fixtures/d60_refuted_noop_routing.json"))
        .expect("D60 fixture");
    (universe, fixture)
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
    let ready = support::array(fixture["inventory"].get("items"));
    let failed = support::array(fixture.get("failed_noop_outcomes"));
    let mut reclassified = BTreeSet::new();

    let route = route_refuted_noops(
        &contract,
        &ready,
        &BTreeSet::new(),
        &failed,
        &mut reclassified,
    );

    let NoopProofExhaustionRoute::ScheduleImplementation(items) = route else {
        panic!("refuted noop must schedule implementation");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["work_type"], "implementation");
    assert_eq!(items[0]["canonical_task_ids"][0], "TASK-TDL-001");
    assert_eq!(items[0]["noop_reclassification"]["count"], 1);
    assert!(
        !items[0]["artifact_requirements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
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
