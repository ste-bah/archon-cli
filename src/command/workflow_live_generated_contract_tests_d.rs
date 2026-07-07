#[test]
fn workflow_live_generated_contract_normalizes_direct_command_retry_fixture() {
    let fixture = include_str!("fixtures/wfc5d4_verification_repair_plan_1_3.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 3);
    assert!(
        inventory.issues.is_empty(),
        "direct command retry items must be canonical before source scheduling: {:?}",
        inventory.issues
    );
    for item in &inventory.items {
        assert_eq!(
            item["canonical_task_ids"],
            serde_json::json!(["TASK-TDL-010"])
        );
        assert!(
            item["focused_verification"]
                .as_array()
                .is_some_and(|items| items.len() == 1)
        );
        assert!(
            item["expected_evidence"]
                .as_array()
                .is_some_and(|items| items.len() == 1)
        );
    }
}

#[test]
fn workflow_live_generated_contract_keeps_consolidated_retry_item_schedulable() {
    let fixture = include_str!("fixtures/wfcac_verification_repair_consolidated_retry_item.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 1);
    assert!(inventory.issues.is_empty(), "{:?}", inventory.issues);
    let item = &inventory.items[0];
    assert_eq!(item["canonical_task_ids"], serde_json::json!(["TASK-TDL-001"]));
    assert!(item["focused_verification"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("Read registry.json"))
        })
    }));
    assert!(item["artifact_requirements"]
        .as_array()
        .is_some_and(|items| items.len() == 2));
}

