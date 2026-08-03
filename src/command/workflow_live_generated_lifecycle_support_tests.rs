use super::*;

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X-020".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-X-020.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
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
fn repair_merge_normalizes_prefix_stripped_canonical_id() {
    let universe = universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let inventory = contract.normalize_inventory(&serde_json::json!({
        "items": [{
            "item_id": "implementation-x",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-X-020"],
            "dependency_ids": [],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["implemented"],
            "focused_verification": ["cargo test focused"],
            "artifact_requirements": [],
        }]
    }));
    let repair = serde_json::json!({
        "items": [{
            "item_id": "implementation-x",
            "canonical_task_ids": ["X-020"],
            "target_files": ["src/repaired.rs"],
        }]
    });

    let merged =
        contract.normalize_inventory(&merge_inventory_repair(&contract, &inventory, &repair));
    let item = &array(merged.get("items"))[0];

    assert_eq!(
        item["canonical_task_ids"],
        serde_json::json!(["TASK-X-020"])
    );
    assert_eq!(item["target_files"], serde_json::json!(["src/repaired.rs"]));
}

#[test]
fn host_result_normalization_stamps_bare_ids_and_surfaces_unknown_ids() {
    let universe = universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let value = contract.normalize_canonical_id_fields(&serde_json::json!({
        "outcomes": [
            {"canonical_task_ids": ["X-020"]},
            {"canonical_task_ids": ["UNKNOWN-020"]},
        ]
    }));

    assert_eq!(
        value["outcomes"][0]["canonical_task_ids"],
        serde_json::json!(["TASK-X-020"])
    );
    assert_eq!(
        value["outcomes"][1]["canonical_id_repair_issues"][0]["unresolved_ids"],
        serde_json::json!(["UNKNOWN-020"])
    );
    assert_eq!(
        value["outcomes"][1]["canonical_task_ids"],
        serde_json::json!([])
    );
}

#[test]
fn fabel_verification_plan_schedules_one_branch_per_item() {
    let contract = fabel_contract();
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wffed_verification_plan_1.json"))
            .expect("fixture json");
    let inventory = contract.normalize_inventory(&value);
    let items = verification_items(&contract, &inventory);
    assert_eq!(items.len(), 4);
    for (scheduled, planned) in items.iter().zip(value["items"].as_array().unwrap()) {
        assert_eq!(scheduled["item_id"], planned["item_id"]);
        assert_eq!(strings_of(scheduled.get("focused_verification")).len(), 1);
    }
}

#[test]
fn fabel_shape_repair_schedules_one_branch_per_item() {
    let contract = fabel_contract();
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wffed_verification_repair_shape_repair_1_1_1.json"
    ))
    .expect("fixture json");
    let inventory = contract.normalize_inventory(&value);
    let items = verification_items(&contract, &inventory);
    assert_eq!(items.len(), 4);
    assert_eq!(strings_of(items[0].get("focused_verification")).len(), 2);
    assert_eq!(strings_of(items[1].get("focused_verification")).len(), 2);
    assert_eq!(strings_of(items[2].get("focused_verification")).len(), 4);
    assert_eq!(strings_of(items[3].get("focused_verification")).len(), 2);
}

#[test]
fn verification_plan_schedules_exactly_one_branch_per_item_id() {
    let contract = fabel_contract();
    let items = (1..=17)
        .map(|index| {
            serde_json::json!({
                "item_id": format!("verify-{index}"),
                "canonical_task_ids": ["TASK-TDL-010"],
                "focused_verification": ["cargo test focused", "inspect artifact"],
                "expected_evidence": ["focused target passes"]
            })
        })
        .collect::<Vec<_>>();
    let inventory = contract.normalize_inventory(&serde_json::json!({ "items": items }));

    let scheduled = verification_items(&contract, &inventory);

    assert_eq!(scheduled.len(), 17);
    assert_eq!(scheduled[0]["item_id"], "verify-1");
    assert_eq!(scheduled[16]["item_id"], "verify-17");

    let rescheduled = split_focused_verification_items(&contract, &scheduled);
    assert_eq!(rescheduled, scheduled);
}

#[test]
fn retry_identity_strips_compounded_check_suffixes() {
    let contract = fabel_contract();
    let inventory = contract.normalize_inventory(&serde_json::json!({
        "items": [{
            "item_id": "foo-check-2-check-10",
            "source_item_id": "foo-check-2",
            "canonical_task_ids": ["TASK-TDL-010"],
            "focused_verification": ["cargo test focused"],
            "source_residual_gap_ids": ["gap-1"],
            "failed_predicate": "focused target must pass"
        }]
    }));

    let scheduled = retry_verification_items(&contract, &inventory);

    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0]["item_id"], "retry-foo");
    assert_eq!(scheduled[0]["source_item_id"], "foo-check-2");
}

#[test]
fn distinct_retry_gaps_share_a_stem_without_becoming_clones() {
    let contract = fabel_contract();
    let inventory = contract.normalize_inventory(&serde_json::json!({
        "items": [{
            "item_id": "foo-check-1",
            "source_item_id": "foo-check-1",
            "canonical_task_ids": ["TASK-TDL-010"],
            "source_residual_gap_ids": ["gap-a"],
            "failed_predicate": "predicate a",
            "focused_verification": ["cargo test focused"]
        }, {
            "item_id": "foo-check-2",
            "source_item_id": "foo-check-2",
            "canonical_task_ids": ["TASK-TDL-010"],
            "source_residual_gap_ids": ["gap-b"],
            "failed_predicate": "predicate b",
            "focused_verification": ["cargo test focused"]
        }]
    }));

    let scheduled = retry_verification_items(&contract, &inventory);

    assert_eq!(scheduled.len(), 2);
    assert_ne!(scheduled[0]["item_id"], scheduled[1]["item_id"]);
    assert!(scheduled.iter().all(|item| {
        item["item_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("retry-foo-variant-") && !id.contains("-check-"))
    }));
}

fn fabel_contract() -> LifecycleContract<'static> {
    LifecycleContract {
        task_universe: Box::leak(Box::new(WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-TDL-010.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            }],
        })),
        target_repository_root: Some("/repo"),
    }
}
