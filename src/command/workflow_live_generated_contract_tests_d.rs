use super::*;

#[test]
fn workflow_live_generated_contract_normalizes_direct_command_retry_fixture() {
    let fixture = include_str!("fixtures/wfc5d4_verification_repair_plan_1_3.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 3);
    assert_eq!(
        inventory
            .issues
            .iter()
            .filter(|issue| issue.kind == GeneratedContractIssueKind::EvidenceRepair)
            .count(),
        6,
        "legacy retries must repair dropped invariant identity"
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
fn workflow_live_generated_contract_requires_invariants_for_consolidated_retry_item() {
    let fixture = include_str!("fixtures/wfcac_verification_repair_consolidated_retry_item.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 1);
    assert_eq!(
        inventory
            .issues
            .iter()
            .filter(|issue| issue.kind == GeneratedContractIssueKind::EvidenceRepair)
            .count(),
        2
    );
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

#[test]
fn workflow_live_generated_contract_flattens_grouped_inventory_items() {
    let value = serde_json::json!({
        "data": {
            "items": {
                "implementation": [{
                    "item_id": "impl-TASK-TDL-010",
                    "work_type": "implementation",
                    "canonical_task_ids": ["TASK-TDL-010"],
                    "dependency_ids": ["TASK-TDL-001"],
                    "target_files": ["src/command/trading_data.rs"],
                    "acceptance_criteria": ["registry v2"],
                    "focused_verification": ["cargo test trading_data"],
                    "artifact_requirements": [".archon/trading-lab/data/registry.json"]
                }],
                "verified_noop": [{
                    "item_id": "noop-TASK-TDL-001",
                    "work_type": "verified_noop",
                    "canonical_task_ids": ["TASK-TDL-001"],
                    "dependency_ids": [],
                    "acceptance_criteria": ["gap audit"],
                    "noop_proof": "gap audit evidence exists",
                    "noop_proof_refs": ["context/progress.md"],
                    "artifact_requirements": []
                }]
            }
        }
    });

    let inventory = normalize_generated_inventory_value(&value, Some(&task_universe()));

    assert_eq!(inventory.items.len(), 2);
    assert!(
        inventory.issues.is_empty(),
        "grouped inventory buckets must be canonical before scheduling: {:?}",
        inventory.issues
    );
    assert!(
        inventory
            .items
            .iter()
            .any(|item| item["work_type"] == "implementation")
    );
    assert!(
        inventory
            .items
            .iter()
            .any(|item| item["work_type"] == "verified_noop")
    );
}
