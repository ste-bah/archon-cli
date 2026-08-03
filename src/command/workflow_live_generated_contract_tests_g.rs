use super::*;

pub(super) fn invariant_chain_fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture")
}

#[test]
fn retry_item_dropping_source_invariant_is_repairable_shape_issue() {
    let fixture = invariant_chain_fixture();
    let inventory = normalize_generated_inventory_value(&fixture["invalid_retry_plan"], None);

    assert!(inventory.issues.iter().any(|issue| {
        issue.kind == GeneratedContractIssueKind::EvidenceRepair
            && issue.field == "source_residual_gap_ids"
    }));
    assert!(inventory.issues.iter().any(|issue| {
        issue.kind == GeneratedContractIssueKind::EvidenceRepair
            && issue.field == "failed_predicate"
    }));
}

#[test]
fn retry_item_preserving_source_invariant_is_contract_valid() {
    let fixture = invariant_chain_fixture();
    let inventory = normalize_generated_inventory_value(&fixture["valid_retry_plan"], None);

    assert!(inventory.issues.is_empty(), "{:?}", inventory.issues);
    assert_eq!(
        inventory.items[0]["source_residual_gap_ids"][0],
        "provider-env-status-mismatch"
    );
    assert_eq!(
        inventory.items[0]["failed_predicate"],
        "Artifact credential status must match current redacted provider proof."
    );
}

#[test]
fn retry_invariant_aliases_normalize_to_canonical_fields() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "retry-item",
            "source_item_id": "failed-item",
            "canonical_task_ids": ["TASK-FIXTURE-001"],
            "retry_type": "verification_evidence_repair",
            "sourceResidualGapIds": ["gap-one"],
            "failurePredicate": "expected value equals observed value",
            "focused_verification": ["compare values"],
            "expected_evidence": ["gap-one resolved"]
        }),
        None,
    );

    assert!(normalized.issues.is_empty(), "{:?}", normalized.issues);
    assert_eq!(normalized.value["source_residual_gap_ids"][0], "gap-one");
    assert_eq!(
        normalized.value["failed_predicate"],
        "expected value equals observed value"
    );
}

#[test]
fn d70_artifact_only_work_without_declared_contract_is_repairable() {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-EX-001".to_string(),
                source_path: "tasks/TASK-EX-001.md".to_string(),
                acceptance_criteria: vec!["Produce the audit report artifact.".to_string()],
                ..Default::default()
            },
        ],
    };
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({"items": [{
            "item_id": "artifact-only-audit",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-EX-001"],
            "dependency_ids": [],
            "target_files": [],
            "acceptance_criteria": ["Produce the audit report artifact."],
            "focused_verification": ["Verify the audit report exists."],
            "artifact_requirements": [],
        }]}),
        Some(&universe),
        Some("/repo"),
    );

    assert!(inventory.issues.iter().any(|issue| {
        issue.kind == GeneratedContractIssueKind::ArtifactRequirementsDiscovery
            && issue.field == "deliverable_contracts"
            && issue.message.contains("task produces artifacts")
    }));
}

#[test]
fn d70_declared_contract_allows_artifact_only_ownership() {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-EX-001".to_string(),
                source_path: "tasks/TASK-EX-001.md".to_string(),
                acceptance_criteria: vec!["Produce the audit report artifact.".to_string()],
                deliverable_contracts: vec![
                super::super::super::workflow_live_task_universe::WorkflowV2DeliverableContract {
                    kind: "audit_report".to_string(),
                    artifact_path: ".archon/reports/current.json".to_string(),
                    ..Default::default()
                },
            ],
                ..Default::default()
            },
        ],
    };
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({"items": [{
            "item_id": "artifact-only-audit",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-EX-001"],
            "dependency_ids": [],
            "target_files": [],
            "acceptance_criteria": ["Produce the audit report artifact."],
            "focused_verification": ["Verify the audit report exists."],
            "artifact_requirements": [".archon/reports/current.json"],
        }]}),
        Some(&universe),
        Some("/repo"),
    );

    assert!(
        !inventory
            .issues
            .iter()
            .any(|issue| { issue.field == "deliverable_contracts" })
    );
}
