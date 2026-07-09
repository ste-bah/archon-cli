use super::*;

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-X-020".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-X-020.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
            },
        ],
    }
}

#[test]
fn repair_merge_accepts_placeholder_artifacts_as_expected_evidence() {
    let universe = universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let inventory = contract.normalize_inventory(&serde_json::json!({
        "items": [{
            "item_id": "impl-validation",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-X-020"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["validation report generation exists"],
            "focused_verification": ["validation report generation test"]
        }]
    }));
    assert_eq!(
        issues_of_kind(&inventory, "artifact_requirements_discovery").len(),
        1
    );

    let repair = serde_json::json!({
        "status": "needs_review",
        "data": {
            "items": [{
                "item_id": "impl-validation",
                "canonical_task_ids": ["TASK-X-020"],
                "artifact_requirements": [
                    ".archon/data/datasets/<dataset-id>/<version>/validation.json"
                ],
                "expected_evidence": ["validation report exists for the created dataset"]
            }]
        }
    });
    let merged = merge_inventory_repair(&contract, &inventory, &repair);
    let normalized = contract.normalize_inventory(&merged);

    assert!(issues_of_kind(&normalized, "artifact_requirements_discovery").is_empty());
    let item = &array(normalized.get("items"))[0];
    assert_eq!(item["artifact_requirements"], serde_json::json!([]));
    assert!(
        strings_of(item.get("expected_evidence"))
            .iter()
            .any(|entry| entry.contains("<dataset-id>"))
    );
}

#[test]
fn fabel_verification_plan_splits_to_four_items() {
    let contract = fabel_contract();
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wffed_verification_plan_1.json"))
            .expect("fixture json");
    let inventory = contract.normalize_inventory(&value);
    let items = verification_items(&contract, &inventory);
    assert_eq!(items.len(), 4);
}

#[test]
fn fabel_shape_repair_splits_to_ten_items() {
    let contract = fabel_contract();
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wffed_verification_repair_shape_repair_1_1_1.json"
    ))
    .expect("fixture json");
    let inventory = contract.normalize_inventory(&value);
    let items = verification_items(&contract, &inventory);
    assert_eq!(items.len(), 10);
}

fn fabel_contract() -> LifecycleContract<'static> {
    LifecycleContract {
        task_universe: Box::leak(Box::new(WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![
                super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                    canonical_task_id: "TASK-TDL-010".to_string(),
                    aliases: Vec::new(),
                    source_path: "tasks/TASK-TDL-010.md".to_string(),
                    dependency_ids: Vec::new(),
                    title: None,
                    artifact_requirements: Vec::new(),
                },
            ],
        })),
        target_repository_root: Some("/repo"),
    }
}
