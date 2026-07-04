#[test]
fn workflow_live_generated_contract_js_helper_is_pure() {
    let helper = generated_prd_contract_js();
    assert!(!helper.contains("w."));
    assert!(!helper.contains("await "));
    assert!(!helper.contains("import("));
    assert!(!helper.contains("eval("));
    assert!(helper.contains("function generatedContractApplyRetryAliases"));
}

#[test]
fn workflow_live_generated_contract_js_splits_broad_verification_items_without_host_calls() {
    let helper = generated_prd_contract_js();
    assert!(helper.contains("function splitFocusedVerificationItems"));
    assert!(helper.contains("focused_verification: [check]"));
    assert!(helper.contains("split_from_item_id"));
    assert!(helper.contains("expected_completion_evidence"));
    assert!(!helper.contains("splitFocusedVerificationItems(items) {\n    await"));
    assert!(!helper.contains("splitFocusedVerificationItems(items) {\n    w."));
}

#[test]
fn workflow_live_generated_contract_js_does_not_split_verification_requirement_bullets() {
    let helper = generated_prd_contract_js();
    assert!(helper.contains(
        r#"copyArray("focused_verification", ["focused_verification", "focusedVerification", "focused_tests", "focusedTests", "verification", "verification_requirements", "verificationRequirements", "verification_shape", "verificationShape", "command", "check", "test_command", "testCommand", "commands", "commands_run", "commandsRun", "manual_fixture_steps", "manualFixtureSteps"])"#
    ));
    assert!(helper.contains(
        r#"generatedContractRawValues(source, ["verification_requirements", "verificationRequirements", "verification", "verification_shape", "verificationShape", "focused_verification""#
    ));
}

#[test]
fn workflow_live_generated_contract_normalizes_expected_completion_evidence() {
    let value = serde_json::json!({
        "item_id": "retry-verify-TASK-TDL-010-artifact-contract-source-check-canonical",
        "canonical_task_ids": ["TASK-TDL-010"],
        "source_item_id": "verify-TASK-TDL-010-artifact-contract-source-check",
        "expected_completion_evidence": {
            "artifact_paths": ["crates/archon-trading/src/data_store.rs"],
            "required_summary_points": ["data_store.rs writes metadata.json"]
        },
        "repair_type": "verification_evidence_shape"
    });
    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;
    assert_eq!(normalized["canonical_task_ids"][0], "TASK-TDL-010");
    assert_eq!(
        normalized["focused_verification"][0],
        "data_store.rs writes metadata.json"
    );
    assert_eq!(
        normalized["artifact_requirements"][0],
        "crates/archon-trading/src/data_store.rs"
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_retry_expected_evidence() {
    let value = serde_json::json!({
        "id": "retry-verify-TASK-TDL-010-focused-compile-check",
        "canonical_task_ids": ["TASK-TDL-010"],
        "commands": ["cargo check -p archon-trading --lib"],
        "expected_evidence": "cargo check exits 0 with no compiler errors"
    });

    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;

    assert_eq!(
        normalized["focused_verification"][0],
        "cargo check -p archon-trading --lib"
    );
    assert_eq!(
        normalized["expected_evidence"][0],
        "cargo check exits 0 with no compiler errors"
    );
    assert!(normalized.get("artifact_requirements").is_none());
}

#[test]
fn workflow_live_generated_contract_normalizes_retry_steps_fixture() {
    let fixture = include_str!("fixtures/wffe12_verification_repair_plan_1_3_items.json");
    let values: Vec<serde_json::Value> = serde_json::from_str(fixture).expect("fixture json");
    let normalized = values
        .iter()
        .map(|value| normalize_generated_item_value(value, Some(&tdl_task_universe())).value)
        .collect::<Vec<_>>();

    assert_eq!(normalized.len(), 2);
    for item in &normalized {
        assert!(
            item["canonical_task_ids"]
                .as_array()
                .is_some_and(|ids| ids.len() == 2)
        );
        assert!(
            item["focused_verification"]
                .as_array()
                .is_some_and(|items| items.len() == 2)
        );
        assert!(
            item["expected_evidence"]
                .as_array()
                .is_some_and(|items| items.len() >= 6)
        );
        assert!(item.get("artifact_requirements").is_none());
        assert_eq!(
            item["source_item_id"], item["source_failed_item_id"],
            "{item:#?}"
        );
    }
}

#[test]
fn workflow_live_generated_contract_extracts_nested_retry_repair_plan_fixture() {
    let fixture = include_str!("fixtures/wff68_verification_repair_plan_1_1.json");
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

#[test]
fn workflow_live_generated_contract_extracts_direct_retry_items_fixture() {
    let fixture = include_str!("fixtures/wf1ca_verification_repair_plan_1_1.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 1);
    let item = &inventory.items[0];
    assert_eq!(
        item["canonical_task_ids"],
        serde_json::json!(["TASK-TDL-001"])
    );
    assert_eq!(
        item["source_item_id"],
        "verification-wave-1-verify-TASK-TDL-001-conditional-workspace-check"
    );
    assert!(
        item["focused_verification"]
            .as_array()
            .is_some_and(|items| items.len() >= 1)
    );
    assert!(
        item["expected_evidence"]
            .as_array()
            .is_some_and(|items| items.len() >= 2)
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_retry_command_fixture() {
    let fixture = include_str!("fixtures/wf19f5_verification_repair_plan_1_1.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 2);
    for item in &inventory.items {
        assert_eq!(
            item["canonical_task_ids"],
            serde_json::json!(["TASK-TDL-010"])
        );
        assert!(
            item["focused_verification"]
                .as_array()
                .is_some_and(|items| items.len() >= 2)
        );
        assert!(
            item["expected_evidence"]
                .as_array()
                .is_some_and(|items| items.len() >= 1)
        );
        assert!(
            item["artifact_requirements"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }
}

#[test]
fn workflow_live_generated_contract_normalizes_nested_result_retry_fixture() {
    let fixture = include_str!("fixtures/wf19f5_verification_repair_plan_1_3.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 1);
    let item = &inventory.items[0];
    assert_eq!(
        item["canonical_task_ids"],
        serde_json::json!(["TASK-TDL-010"])
    );
    assert_eq!(item["item_id"], "retry-verify-cli-parse-prd-commands");
    assert_eq!(
        item["source_item_id"],
        "verification-wave-1-2-verify-cli-parse-prd-commands"
    );
    assert!(
        item["focused_verification"]
            .as_array()
            .is_some_and(|items| items.len() == 1)
    );
    assert!(
        item["expected_evidence"]
            .as_array()
            .is_some_and(|items| items.len() == 2)
    );
    assert!(
        inventory.issues.is_empty(),
        "retry fixture should not require source metadata repair: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_extracts_nested_repair_items() {
    let value = serde_json::json!({
        "data": {
            "items": {
                "repaired_items": [{
                    "item_id": "impl-TASK-TDL-010",
                    "item_type": "implementation",
                    "canonical_task_ids": ["TASK-TDL-010"],
                    "dependency_ids": ["TASK-TDL-001"],
                    "target_files": ["src/command/trading_data.rs"],
                    "acceptance_criteria": ["registry v2"],
                    "focused_verification": ["cargo test trading_data"],
                    "artifact_requirements": [".archon/trading-lab/data/registry.json"]
                }]
            },
            "verified_noop_items": [{
                "item_id": "noop-TASK-TDL-001",
                "item_type": "verified_noop",
                "canonical_task_ids": ["TASK-TDL-001"],
                "noop_proof": "gap audit exists",
                "noop_proof_refs": ["context/progress.md"],
                "acceptance_criteria": ["gap audit"],
                "artifact_requirements": []
            }]
        }
    });
    let inventory = normalize_generated_inventory_value(&value, Some(&task_universe()));

    assert_eq!(inventory.items.len(), 2);
    assert!(
        inventory.issues.is_empty(),
        "nested repair items should satisfy the contract: {:?}",
        inventory.issues
    );
    let implementation = inventory
        .items
        .iter()
        .find(|item| item["item_id"] == "impl-TASK-TDL-010")
        .expect("implementation item");
    let noop = inventory
        .items
        .iter()
        .find(|item| item["item_id"] == "noop-TASK-TDL-001")
        .expect("noop item");
    assert_eq!(implementation["work_type"], "implementation");
    assert_eq!(noop["work_type"], "verified_noop");
}

#[test]
fn workflow_live_generated_contract_top_level_items_are_authoritative() {
    let value = serde_json::json!({
        "items": [{
            "item_id": "impl-TASK-TDL-010-repaired",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["TASK-TDL-001"],
            "target_files": ["src/command/trading_data.rs"],
            "acceptance_criteria": ["registry v2"],
            "focused_verification": ["cargo test trading_data"],
            "artifact_requirements": [".archon/trading-lab/data/registry.json"]
        }, {
            "item_id": "noop-TASK-TDL-001-repaired",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-TDL-001"],
            "dependency_ids": [],
            "acceptance_criteria": ["gap audit"],
            "noop_proof": "gap audit accepted as prerequisite",
            "noop_proof_refs": ["context/progress.md"],
            "artifact_requirements": []
        }],
        "data": {
            "items": [{
                "item_id": "impl-TASK-TDL-010-stale",
                "work_type": "implementation",
                "canonical_task_ids": ["TASK-TDL-010"],
                "dependency_ids": ["TASK-TDL-001"],
                "target_files": ["src/command/trading_data.rs"],
                "acceptance_criteria": ["stale registry v2"],
                "focused_verification": ["stale cargo test trading_data"],
                "artifact_requirements": [".archon/trading-lab/data/registry.json"]
            }],
            "unresolved_issues": [{
                "kind": "dependency_graph_repair",
                "message": "stale nested issue"
            }]
        }
    });

    let inventory = normalize_generated_inventory_value(&value, Some(&task_universe()));

    assert_eq!(inventory.items.len(), 2);
    assert_eq!(inventory.items[0]["item_id"], "impl-TASK-TDL-010-repaired");
    assert!(
        inventory.issues.is_empty(),
        "stale nested roots must not create duplicate graph issues: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_js_preserves_host_unresolved_issues() {
    let helper = generated_prd_contract_js();
    assert!(helper.contains("const sourceIssues = generatedContractInventorySourceIssues(source)"));
    assert!(helper.contains("function generatedContractInventorySourceIssues"));
    assert!(helper.contains("[\"result\", \"data\"]"));
    assert!(helper.contains("const unresolved_issues = sourceIssues.concat"));
}

#[test]
fn workflow_live_generated_contract_js_merge_repair_is_non_destructive() {
    let helper = generated_prd_contract_js();
    assert!(helper.contains("function mergeInventoryRepair"));
    assert!(helper.contains("const existingItems"));
    assert!(helper.contains("mergedByKey"));
    assert!(helper.contains("return { ...inventory, unresolved_issues: [], items: order.map"));
    assert!(
        !helper.contains("return { ...inventory, items: repairItems };"),
        "repair output must not replace the whole inventory"
    );
}

#[test]
fn workflow_live_generated_contract_wf90070_fixture_normalizes_without_malformed_inventory() {
    let fixture = include_str!("fixtures/wf90070_canonical_inventory.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&task_universe()));
    assert!(
        inventory.issues.is_empty(),
        "fixture should normalize without repair issues: {:?}",
        inventory.issues
    );
    assert_eq!(inventory.items.len(), 2);
    let noop = &inventory.items[0];
    assert_eq!(
        noop.get("noop_proof_refs")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let implementation = &inventory.items[1];
    assert_eq!(
        implementation
            .get("dependency_ids")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(serde_json::Value::as_str),
        Some("TASK-TDL-001")
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_wf580_review_remediation_fixture() {
    let fixture = include_str!("fixtures/wf580_review_remediation_inventory_items.json");
    let values: Vec<serde_json::Value> = serde_json::from_str(fixture).expect("fixture json");
    let normalized = values
        .iter()
        .map(|value| normalize_generated_item_value(value, Some(&tdl_task_universe())).value)
        .collect::<Vec<_>>();

    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0]["item_id"], "REM-TDL-010");
    assert_eq!(normalized[0]["canonical_task_ids"][0], "TASK-TDL-010");
    assert_eq!(normalized[0]["dependency_ids"][0], "TASK-TDL-001");
    assert!(normalized[0]["source_item_id"].as_str().is_some());
    assert_eq!(normalized[0]["failure_status"], "needs_review");
    assert!(
        normalized[0]["failure_evidence"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
    );
    assert!(normalized[0]["required_fix"].as_str().is_some());
    assert!(
        normalized[0]["target_files"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
    );
    assert!(
        normalized[0]["focused_verification"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
    );
    assert!(
        normalized[0]["artifact_requirements"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
    );
    assert_eq!(
        normalized[1]["dependency_ids"][0],
        "__unresolved__:REM-TDL-010"
    );
}

#[test]
fn workflow_live_generated_contract_normalizes_wf580_review_verification_fixture() {
    let fixture = include_str!("fixtures/wf580_review_verification_plan_items.json");
    let values: Vec<serde_json::Value> = serde_json::from_str(fixture).expect("fixture json");
    let normalized = values
        .iter()
        .map(|value| normalize_generated_item_value(value, Some(&tdl_task_universe())).value)
        .collect::<Vec<_>>();

    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0]["item_id"], "VERIFY-TDL-010");
    assert_eq!(normalized[0]["canonical_task_ids"][0], "TASK-TDL-010");
    assert_eq!(normalized[0]["dependency_ids"][0], "TASK-TDL-001");
    assert!(
        normalized[0]["focused_verification"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str() == Some("registry schema migration"))
            })
    );
    assert!(
        normalized[0]["artifact_requirements"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str() == Some(".archon/trading-lab/data/registry.json"))
            })
    );
    assert_eq!(
        normalized[1]["dependency_ids"][0],
        "__unresolved__:VERIFY-TDL-010"
    );
}
