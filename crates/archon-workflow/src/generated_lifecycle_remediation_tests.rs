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

/// The live loop: a verification-triage inventory carries the ROUTED shape
/// (`implementation_failures` / `retry_items`) and never mints `items`. Keying
/// readiness on `items` alone read it as "not ready", so the router regenerated
/// the inventory, triage returned the same shape, and the run cycled until the
/// repair cap — re-deriving the same actionable failure every pass and running
/// no write wave. Observed on wf-b40de9ee: five cycles, three hours, one
/// TASK-TDL-030 failure that already had target files and a required fix.
#[test]
fn a_routed_triage_inventory_is_ready_without_an_items_array() {
    let routed = serde_json::json!({
        "implementation_failures": [{
            "item_id": "remediation-tdl030-ac08-allowlist-removal",
            "canonical_task_ids": ["TASK-TDL-030"],
            "target_files": ["crates/archon-trading/src/data_lake/contracts.rs"],
            "required_fix": "Remove legacy free function; wire ProviderDispatcher."
        }],
        "retry_items": [],
        "terminal_blockers": []
    });

    assert!(
        super::remediation_inventory_ready(&routed),
        "a routed inventory with actionable failures must be ready"
    );
}

#[test]
fn retry_items_alone_are_also_work() {
    let routed = serde_json::json!({
        "implementation_failures": [],
        "retry_items": [{"item_id": "retry-1", "canonical_task_ids": ["TASK-A"]}]
    });

    assert!(super::remediation_inventory_ready(&routed));
}

/// The classic wave-shaped inventory keeps working exactly as before.
#[test]
fn an_items_shaped_inventory_is_still_ready() {
    let wave = serde_json::json!({ "items": [{"item_id": "r1"}] });

    assert!(super::remediation_inventory_ready(&wave));
}

/// Nothing to do is still not ready — this must not become "always ready".
#[test]
fn an_empty_inventory_is_not_ready() {
    for empty in [
        serde_json::json!({}),
        serde_json::json!({"items": []}),
        serde_json::json!({"implementation_failures": [], "retry_items": []}),
    ] {
        assert!(
            !super::remediation_inventory_ready(&empty),
            "empty inventory must not be ready: {empty}"
        );
    }
}

/// Unresolved issues still gate BOTH shapes.
#[test]
fn unresolved_issues_block_either_shape() {
    for blocked in [
        serde_json::json!({"items": [{"item_id": "r1"}], "unresolved_issues": ["x"]}),
        serde_json::json!({
            "implementation_failures": [{"item_id": "r1"}],
            "unresolved_issues": ["x"]
        }),
    ] {
        assert!(
            !super::remediation_inventory_ready(&blocked),
            "unresolved issues must block: {blocked}"
        );
    }
}
