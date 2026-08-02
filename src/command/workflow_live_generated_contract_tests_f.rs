use super::*;

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

#[test]
fn target_file_issue_rejects_instruction_text() {
    let issue = super::super::lifecycle_target_file_issue(
        "Produce or update admissible evidence references for the audit",
        Some("/repo"),
    )
    .expect("prose target must raise a contract issue");
    assert!(issue.contains("not instruction text"), "issue: {issue}");
}

#[test]
fn target_file_issue_accepts_repo_relative_path() {
    assert!(
        super::super::lifecycle_target_file_issue("crates/archon-core/src/lib.rs", Some("/repo"))
            .is_none()
    );
}

#[test]
fn artifact_requirements_prose_moves_to_expected_evidence() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "impl-artifact-guidance",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["evidence is complete"],
            "focused_verification": ["cargo test evidence"],
            "artifact_requirements": [
                "Implementation evidence must include exact focused command output and changed source file references."
            ]
        }),
        Some(&task_universe()),
    )
    .value;

    assert_eq!(normalized["artifact_requirements"], serde_json::json!([]));
    assert_eq!(
        normalized["expected_evidence"][0],
        "Implementation evidence must include exact focused command output and changed source file references."
    );
}

#[test]
fn artifact_requirements_keep_concrete_paths_and_path_objects() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "impl-artifact-paths",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["evidence is complete"],
            "focused_verification": ["cargo test evidence"],
            "artifact_requirements": [
                ".archon/artifacts/evidence.json",
                {"path": "artifacts/report.json", "description": "report"}
            ]
        }),
        Some(&task_universe()),
    )
    .value;

    assert_eq!(
        normalized["artifact_requirements"][0],
        ".archon/artifacts/evidence.json"
    );
    assert_eq!(normalized["artifact_requirements"][1]["path"], "artifacts/report.json");
    assert!(normalized.get("expected_evidence").is_none());
}

#[test]
fn artifact_requirements_glob_patterns_move_to_expected_evidence() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "impl-artifact-patterns",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["evidence is complete"],
            "focused_verification": ["cargo test evidence"],
            "artifact_requirements": [
                ".archon/trading-lab/data/datasets/*/*/validation.json",
                {"path": ".archon/trading-lab/data/datasets/*/*/validation-report.json"}
            ]
        }),
        Some(&task_universe()),
    )
    .value;

    assert_eq!(normalized["artifact_requirements"], serde_json::json!([]));
    assert_eq!(
        normalized["expected_evidence"],
        serde_json::json!([
            ".archon/trading-lab/data/datasets/*/*/validation.json",
            ".archon/trading-lab/data/datasets/*/*/validation-report.json"
        ])
    );
}

#[test]
fn artifact_requirements_placeholder_fixture_moves_to_expected_evidence() {
    let fixture = include_str!("fixtures/wffe96_artifact_requirements_discovery_3_items.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&placeholder_task_universe()));

    assert!(inventory.issues.is_empty(), "issues: {:?}", inventory.issues);
    assert_eq!(inventory.items.len(), 2);
    for item in inventory.items {
        assert_eq!(item["artifact_requirements"], serde_json::json!([]));
        assert!(
            item["expected_evidence"]
                .as_array()
                .is_some_and(|items| items.iter().any(|entry| entry
                    .as_str()
                    .is_some_and(|text| text.contains('<')))),
            "item should retain placeholder as evidence: {item}"
        );
    }
}

#[test]
fn artifact_requirements_malformed_object_is_repairable_issue() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "impl-artifact-malformed",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["evidence is complete"],
            "focused_verification": ["cargo test evidence"],
            "artifact_requirements": [
                {"label": "report", "kind": "summary"}
            ]
        }),
        Some(&task_universe()),
    );

    assert!(
        normalized
            .issues
            .iter()
            .any(|issue| issue.kind == GeneratedContractIssueKind::ArtifactRequirementsDiscovery),
        "ambiguous artifact objects must be repaired before scheduling: {:?}",
        normalized.issues
    );
}

#[test]
fn fabel_verification_plan_items_normalize_idempotently() {
    assert_fixture_items_are_idempotent("fixtures/wffed_verification_plan_1.json");
}

#[test]
fn fabel_shape_repair_items_normalize_idempotently() {
    assert_fixture_items_are_idempotent("fixtures/wffed_verification_repair_shape_repair_1_1_1.json");
}

#[test]
fn fabel_triage_retry_items_normalize_idempotently() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wffed_verification_failure_triage_1_2.json"
    ))
    .expect("fixture json");
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));
    assert_eq!(inventory.items.len(), 3);
    for item in inventory.items {
        assert!(
            item["focused_verification"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "triage retry item must normalize recommended_retry into focused_verification: {item}"
        );
        let once = normalize_generated_item_value(&item, Some(&tdl_task_universe())).value;
        let twice = normalize_generated_item_value(&once, Some(&tdl_task_universe())).value;
        assert_eq!(once, twice);
    }
}

pub(super) fn assert_fixture_items_are_idempotent(fixture: &str) {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wffed_verification_plan_1.json"))
            .expect("fixture json");
    let value = if fixture.ends_with("shape_repair_1_1_1.json") {
        serde_json::from_str(include_str!(
            "fixtures/wffed_verification_repair_shape_repair_1_1_1.json"
        ))
        .expect("shape fixture json")
    } else {
        value
    };
    let inventory = normalize_generated_inventory_value(&value, Some(&tdl_task_universe()));
    assert_eq!(inventory.items.len(), 4);
    for item in inventory.items {
        let once = normalize_generated_item_value(&item, Some(&tdl_task_universe())).value;
        let twice = normalize_generated_item_value(&once, Some(&tdl_task_universe())).value;
        assert_eq!(once, twice);
    }
}

pub(super) fn placeholder_task_universe() -> super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverse {
    super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-X-020".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-X-020.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-X-130".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-X-130.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
        ],
    }
}
