use super::*;

fn test_universe() -> crate::task_universe::WorkflowV2TaskUniverse {
    crate::task_universe::WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![crate::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X-001".to_string(),
            aliases: Vec::new(),
            source_path: "/tmp/TASK-X-001.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    }
}

#[test]
fn followup_remediation_preserves_failure_context_from_source() {
    let universe = test_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let source = serde_json::json!({
        "item_id": "rem-source",
        "canonical_task_ids": ["TASK-X-001"],
        "dependency_ids": [],
        "target_files": ["src/lib.rs"],
        "failure_status": "failed",
        "failure_evidence": ["declared project artifact missing"],
        "required_fix": ["produce concrete artifact evidence"],
        "focused_verification": ["cargo test artifact_contract"],
        "artifact_requirements": []
    });
    let raw = serde_json::json!({
        "items": [{
            "item_id": "rem-followup",
            "canonical_task_ids": ["TASK-X-001"],
            "dependency_ids": [],
            "target_files": [],
            "required_fix": ["repair artifact evidence"],
            "focused_verification": ["cargo test artifact_contract"],
            "artifact_requirements": []
        }]
    });

    let normalized = normalize_remediation_inventory_for_sources(
        &contract,
        &raw,
        &[source],
        &[],
        "remediation-wave-1",
    );

    let item = &normalized["items"][0];
    assert_eq!(item["source_item_id"], "rem-source");
    assert_eq!(item["failure_status"], "failed");
    assert_eq!(
        item["failure_evidence"][0],
        "declared project artifact missing"
    );
    let issues = array(normalized.get("unresolved_issues"));
    assert!(
        issues.iter().all(|issue| issue["field"] != "failure_status"
            && issue["field"] != "failure_evidence"),
        "normalized: {}",
        serde_json::to_string_pretty(&normalized).expect("json")
    );
}
