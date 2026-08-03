use super::*;

#[test]
fn workflow_live_generated_contract_normalizes_proof_references_to_noop_refs() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "id": "noop-001",
            "canonical_task_id": "T001",
            "dependencies": [],
            "work_type": "verified_noop",
            "acceptanceCriteria": ["criterion"],
            "noop_proof": "already implemented",
            "proof_references": ["src/lib.rs:10"],
            "artifact_requirements": []
        }),
        Some(&task_universe()),
    );
    assert_eq!(
        normalized
            .value
            .get("canonical_task_ids")
            .and_then(serde_json::Value::as_array)
            .and_then(|values| values.first())
            .and_then(serde_json::Value::as_str),
        Some("TASK-TDL-001")
    );
    assert_eq!(
        normalized
            .value
            .get("noop_proof_refs")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert!(normalized.issues.is_empty());
}

#[test]
fn workflow_live_generated_contract_classifies_repairable_inventory_gaps() {
    let inventory = normalize_generated_inventory_value(
        &serde_json::json!({
            "items": [{
                "id": "impl-010",
                "task_ids": ["T010"],
                "depends_on": ["T001"],
                "work_type": "implementation",
                "acceptance": ["criterion"]
            }]
        }),
        Some(&task_universe()),
    );
    let kinds = inventory
        .issues
        .iter()
        .map(|issue| issue.kind.as_str())
        .collect::<BTreeSet<_>>();
    assert!(kinds.contains("target_file_discovery"));
    assert!(kinds.contains("verification_requirements_discovery"));
    assert!(kinds.contains("artifact_requirements_discovery"));
}

#[test]
fn workflow_live_generated_contract_empty_inventory_routes_to_shape_repair() {
    let inventory = normalize_generated_inventory_value(
        &serde_json::json!({
            "status": "needs_review",
            "summary": "audit evidence without schedulable implementation items",
            "task_coverage": [],
            "residual_gaps": []
        }),
        Some(&task_universe()),
    );
    assert!(inventory.items.is_empty());
    assert!(inventory.issues.iter().any(|issue| {
        issue.kind.as_str() == "inventory_shape_repair" && issue.field == "items"
    }));
}

#[test]
fn workflow_live_generated_contract_ignores_unowned_support_items_for_scheduling() {
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({
            "items": [
                {
                    "id": "impl-001",
                    "canonical_task_ids": ["TASK-TDL-001"],
                    "dependencies": [],
                    "work_type": "implementation",
                    "target_files": ["src/lib.rs"],
                    "acceptance_criteria": ["criterion"],
                    "focused_verification": ["cargo test -p demo"],
                    "artifact_requirements": []
                },
                {
                    "id": "crosscutting-verification-support",
                    "canonical_task_ids": [],
                    "dependency_ids": ["TASK-TDL-001"],
                    "work_type": "verification_support",
                    "proof_refs": ["support evidence only"]
                }
            ]
        }),
        Some(&WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: vec!["/tmp/tasks".to_string()],
            tasks: vec![
                super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                    canonical_task_id: "TASK-TDL-001".to_string(),
                    aliases: vec!["T001".to_string()],
                    source_path: "/tmp/TASK-TDL-001.md".to_string(),
                    dependency_ids: Vec::new(),
                    title: None,
                    artifact_requirements: Vec::new(),
                    ..Default::default()
                },
            ],
        }),
        Some("/tmp/repo"),
    );

    assert_eq!(
        inventory.items.len(),
        1,
        "support records must not become schedulable inventory items"
    );
    assert!(
        inventory.issues.is_empty(),
        "unowned support evidence must not produce malformed inventory issues: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_wf6cc_repair_fixture_routes_gaps_to_investigation() {
    let fixture = include_str!("fixtures/wf6cc_dependency_graph_repair_2.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value_with_repo(
        &value,
        Some(&tdl_task_universe()),
        Some("/Volumes/Externalwork/archon-cli/archon-cli"),
    );
    let kinds = inventory
        .issues
        .iter()
        .map(|issue| issue.kind.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        inventory.items.len(),
        15,
        "the crosscutting verification_support record must be filtered out"
    );
    assert!(
        !kinds.contains("inventory_shape_repair"),
        "support records must not keep the inventory in malformed shape repair: {:?}",
        inventory.issues
    );
    assert!(
        kinds.contains("target_file_discovery"),
        "skeleton implementation items must route to target investigation"
    );
    assert!(
        kinds.contains("verification_requirements_discovery"),
        "skeleton implementation items must route to verification investigation"
    );
    assert!(
        kinds.contains("artifact_requirements_discovery"),
        "skeleton implementation items must route to artifact investigation"
    );
    assert!(
        !kinds.contains("evidence_repair"),
        "proof_refs aliases should canonicalize to noop_proof_refs for the verified_noop item: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_rejects_out_of_repo_implementation_targets() {
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({
            "items": [{
                "id": "impl-001",
                "task_ids": ["T001"],
                "dependencies": [],
                "work_type": "implementation",
                "target_files": [
                    "/tmp/project/tasks/PRD/context/progress.md"
                ],
                "acceptance_criteria": ["criterion"],
                "focused_verification": ["cargo test -p demo"],
                "artifact_requirements": []
            }]
        }),
        Some(&task_universe()),
        Some("/tmp/repo"),
    );

    assert!(inventory.issues.iter().any(|issue| {
        issue.kind.as_str() == "target_file_discovery"
            && issue.message.contains("outside target repository root")
    }));
}

#[test]
fn workflow_live_generated_contract_accepts_repo_owned_targets() {
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({
            "items": [{
                "id": "impl-001",
                "task_ids": ["T001"],
                "dependencies": [],
                "work_type": "implementation",
                "target_files": [
                    "/tmp/repo/crates/archon-trading/src/data_lake.rs",
                    "src/command/trading_data.rs"
                ],
                "acceptance_criteria": ["criterion"],
                "focused_verification": ["cargo test -p demo"],
                "artifact_requirements": []
            }]
        }),
        Some(&task_universe()),
        Some("/tmp/repo"),
    );

    assert!(
        !inventory
            .issues
            .iter()
            .any(|issue| issue.kind.as_str() == "target_file_discovery"),
        "repo-owned absolute and relative targets should not produce target issues: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_flags_wf6c30_deadlock_inventory_graph() {
    let fixture = include_str!("fixtures/wf6c30_deadlock_inventory.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let inventory = normalize_generated_inventory_value_with_repo(
        &value,
        Some(&tdl_task_universe()),
        Some("/Volumes/Externalwork/archon-cli/archon-cli"),
    );
    let graph_issues = inventory
        .issues
        .iter()
        .filter(|issue| issue.kind.as_str() == "dependency_graph_repair")
        .collect::<Vec<_>>();

    assert!(
        graph_issues.iter().any(|issue| issue
            .message
            .contains("canonical task 'TASK-TDL-001' is not represented")),
        "missing prerequisite task must be represented as implementation/no-op: {graph_issues:?}"
    );
    assert!(
        graph_issues.iter().any(|issue| {
            issue.item_id.as_deref() == Some("TDL-STORAGE-VALIDATION-BACKTEST-GATES")
                && issue.message.contains("TASK-TDL-010")
                && issue.message.contains("same item")
        }),
        "self/internal dependency overlap must be flagged: {graph_issues:?}"
    );
    assert!(
        graph_issues.iter().any(|issue| {
            issue.item_id.as_deref() == Some("AHDM-EVIDENCE-SPEC-PINE-BACKTEST-READINESS")
                && issue.message.contains("TASK-TDL-100")
                && issue.message.contains("same item")
        }),
        "late grouped self dependency must be flagged: {graph_issues:?}"
    );
}

#[test]
fn workflow_live_generated_contract_accepts_dependency_ordered_inventory_graph() {
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({
            "items": [
                {
                    "id": "noop-001",
                    "canonical_task_ids": ["TASK-TDL-001"],
                    "dependency_ids": [],
                    "work_type": "verified_noop",
                    "acceptance_criteria": ["foundation already accepted"],
                    "noop_proof": "existing acceptance report proves foundation",
                    "noop_proof_refs": ["tasks/context/progress.md"],
                    "artifact_requirements": []
                },
                {
                    "id": "impl-010",
                    "canonical_task_ids": ["TASK-TDL-010"],
                    "dependency_ids": ["TASK-TDL-001"],
                    "work_type": "implementation",
                    "target_files": ["src/lib.rs"],
                    "acceptance_criteria": ["registry migration passes"],
                    "focused_verification": ["cargo test registry_migration"],
                    "artifact_requirements": []
                }
            ]
        }),
        Some(&task_universe()),
        Some("/tmp/repo"),
    );

    assert!(
        inventory.issues.is_empty(),
        "ordered no-op then implementation inventory should pass: {:?}",
        inventory.issues
    );
}

#[test]
fn workflow_live_generated_contract_rejects_duplicate_task_assignment() {
    let inventory = normalize_generated_inventory_value_with_repo(
        &serde_json::json!({
            "items": [
                {
                    "id": "impl-010-a",
                    "canonical_task_ids": ["TASK-TDL-010"],
                    "dependency_ids": ["TASK-TDL-001"],
                    "work_type": "implementation",
                    "target_files": ["src/a.rs"],
                    "acceptance_criteria": ["criterion"],
                    "focused_verification": ["cargo test a"],
                    "artifact_requirements": []
                },
                {
                    "id": "impl-010-b",
                    "canonical_task_ids": ["TASK-TDL-010"],
                    "dependency_ids": ["TASK-TDL-001"],
                    "work_type": "implementation",
                    "target_files": ["src/b.rs"],
                    "acceptance_criteria": ["criterion"],
                    "focused_verification": ["cargo test b"],
                    "artifact_requirements": []
                }
            ]
        }),
        Some(&task_universe()),
        Some("/tmp/repo"),
    );

    assert!(
        inventory
            .issues
            .iter()
            .any(|issue| issue.kind.as_str() == "dependency_graph_repair"
                && issue.message.contains("multiple inventory items")),
        "duplicate task assignment must be a graph repair issue: {:?}",
        inventory.issues
    );
}
