use super::*;

#[test]
fn workflow_live_generated_contract_normalizes_retry_plan_fixture() {
    let fixture = archon_test_support::fixtures::WF0ECA_VERIFICATION_REPAIR_PLAN_1_1_ITEM;
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;

    assert_eq!(
        normalized["canonical_task_ids"],
        serde_json::json!(["TASK-TDL-010"])
    );
    assert!(
        normalized["focused_verification"]
            .as_array()
            .is_some_and(|items| items.len() == 1)
    );
    assert!(
        normalized["expected_evidence"]
            .as_array()
            .is_some_and(|items| items.len() >= 2)
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_required_evidence_fixture() {
    let fixture = archon_test_support::fixtures::WF0ECA_VERIFICATION_REPAIR_PLAN_1_2_ITEM;
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;

    assert_eq!(
        normalized["canonical_task_ids"],
        serde_json::json!(["TASK-TDL-010"])
    );
    assert_eq!(
        normalized["focused_verification"][0],
        "cargo test trading_data_prd_commands_parse"
    );
    assert_eq!(
        normalized["expected_evidence"][0],
        "Command exits 0 and output identifies trading_data_prd_commands_parse as passed."
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_provider_env_requirements() {
    let value = serde_json::json!({
        "item_id": "verify-provider",
        "canonical_task_ids": ["TASK-TDL-030"],
        "focused_verification": ["provider native candle check"],
        "required_env_keys": ["POLYGON_API_KEY", "tiingo_token"]
    });

    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;

    assert_eq!(
        normalized["provider_env_requirements"],
        serde_json::json!(["POLYGON_API_KEY", "TIINGO_TOKEN"])
    );
    assert!(
        normalized["expected_evidence"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .any(|item| item == "provider_env_proof:POLYGON_API_KEY"))
    );
}

#[test]
fn workflow_live_generated_contract_provider_required_can_request_preflight() {
    let value = serde_json::json!({
        "items": [{
            "item_id": "verify-provider",
            "canonical_task_ids": ["TASK-TDL-030"],
            "work_type": "implementation",
            "target_files": ["src/provider.rs"],
            "acceptance_criteria": ["native provider check"],
            "provider_required": true,
            "required_env_keys": ["POLYGON_API_KEY"]
        }]
    });

    let normalized = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert!(
        normalized.issues.iter().all(|issue| {
            issue.kind != GeneratedContractIssueKind::ProviderEnvironmentDiscovery
        }),
        "{:#?}",
        normalized.issues
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_code_agnostic_ownership_fields() {
    let value = serde_json::json!({
        "item_id": "impl-owned-paths",
        "canonical_task_ids": ["TASK-TDL-010"],
        "work_type": "implementation",
        "owned_source_files": ["src/lib.rs"],
        "owned_manifest_files": ["Cargo.toml"],
        "owned_lockfiles": ["Cargo.lock"],
        "project_artifact_requirements": [".archon/workflows/wf-example/report.json"],
        "acceptance_criteria": ["declared paths are owned"],
        "focused_verification": ["cargo check"]
    });

    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;
    let targets = normalized["target_files"].as_array().expect("targets");

    assert!(targets.iter().any(|value| value == "src/lib.rs"));
    assert!(targets.iter().any(|value| value == "Cargo.toml"));
    assert!(targets.iter().any(|value| value == "Cargo.lock"));
    assert_eq!(
        normalized["artifact_requirements"][0],
        ".archon/workflows/wf-example/report.json"
    );
}

#[test]
fn workflow_live_generated_contract_embeds_lowercase_task_ids_from_retry_items() {
    let fixture = archon_test_support::fixtures::WF199_VERIFICATION_REPAIR_PLAN_1_1;
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 3);
    for item in &inventory.items {
        assert_eq!(
            item["canonical_task_ids"],
            serde_json::json!(["TASK-TDL-001"])
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
