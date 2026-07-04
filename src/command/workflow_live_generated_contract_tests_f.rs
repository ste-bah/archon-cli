#[test]
fn workflow_live_generated_contract_verified_noop_requires_artifact_metadata() {
    let inventory = normalize_generated_inventory_value(
        &serde_json::json!({
            "items": [{
                "item_id": "noop-foundation",
                "work_type": "verified_noop",
                "canonical_task_ids": ["TASK-TDL-001"],
                "dependency_ids": [],
                "acceptance_criteria": ["foundation evidence exists"],
                "noop_proof": "prior report says foundation is complete",
                "noop_proof_refs": ["context/progress.md"]
            }]
        }),
        Some(&task_universe()),
    );

    assert!(
        inventory
            .issues
            .iter()
            .any(|issue| issue.kind == GeneratedContractIssueKind::ArtifactRequirementsDiscovery),
        "verified_noop items must declare artifact requirements or an explicit empty set: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_verified_noop_accepts_explicit_empty_artifacts() {
    let inventory = normalize_generated_inventory_value(
        &serde_json::json!({
            "items": [{
                "item_id": "noop-foundation",
                "work_type": "verified_noop",
                "canonical_task_ids": ["TASK-TDL-001"],
                "dependency_ids": [],
                "acceptance_criteria": ["foundation evidence exists"],
                "noop_proof": "prior report says foundation is complete",
                "noop_proof_refs": ["context/progress.md"],
                "artifact_requirements": []
            }, {
                "item_id": "impl-dependent",
                "work_type": "implementation",
                "canonical_task_ids": ["TASK-TDL-010"],
                "dependency_ids": ["TASK-TDL-001"],
                "target_files": ["src/lib.rs"],
                "acceptance_criteria": ["dependent work is implemented"],
                "focused_verification": ["cargo test dependent"],
                "artifact_requirements": []
            }]
        }),
        Some(&task_universe()),
    );

    assert!(
        inventory.issues.is_empty(),
        "explicit no-artifact no-op item should remain schedulable: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_flags_noncanonical_remediation_fixture() {
    let fixture = include_str!("fixtures/wfd009_remediation_inventory_invalid_task.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));

    assert_eq!(inventory.items.len(), 1);
    assert_eq!(
        inventory.items[0]["focused_verification"],
        serde_json::json!(["run the focused bin-level verification"])
    );
    assert!(
        inventory
            .issues
            .iter()
            .any(|issue| issue.kind == GeneratedContractIssueKind::TaskUniverseReconcile),
        "noncanonical PRD ids must be repaired before source scheduling: {:?}",
        inventory.issues
    );
}
