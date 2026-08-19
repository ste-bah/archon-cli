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

/// A verification triage answers in the ROUTED shape — `implementation_failures`
/// / `retry_items` / `superseded_items` / `terminal_blockers`, nested under
/// `data`, with no `items` array anywhere.
///
/// The generic collector got this exactly backwards: it never looked at
/// `implementation_failures`, so the one actionable write item was dropped, and
/// it did collect `retry_items`, so a read-only re-verification batch was judged
/// against write-item rules (target_files / required_fix / artifact_requirements)
/// that a retry batch has no business carrying. The unresolved issues that
/// produced were permanent, so the inventory was never ready, the router
/// regenerated it, and triage returned the identical shape — five cycles over
/// three hours on wf-b40de9ee with no write wave ever scheduled.
#[test]
fn a_routed_triage_inventory_yields_its_write_failures_and_drops_the_retry_batch() {
    let universe = test_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let routed = serde_json::json!({
        "status": "accepted",
        "data": {
            "implementation_failures": [{
                "item_id": "remediation-allowlist-removal",
                "source_item_id": "implementation-refuted-noop",
                "canonical_task_ids": ["TASK-X-001"],
                "dependency_ids": [],
                "target_files": ["crates/x/src/contracts.rs"],
                "failure_status": "needs_review",
                "failure_evidence": ["legacy free function still reachable"],
                "required_fix": ["route production callers through the dispatcher"],
                "focused_verification": ["cargo test -p x capability"],
                "artifact_requirements": ["crates/x/src/contracts.rs"]
            }],
            "retry_items": [{
                "item_id": "structural-integrity-cargo-batch",
                "source_item_id": "implementation-refuted-noop",
                "canonical_task_ids": ["TASK-X-001"],
                "failure_status": "needs_review",
                "failure_evidence": "compilation not confirmed this round",
                "focused_verification": ["cargo check -p x --tests"],
                "verification_strategy": "test_execution"
            }],
            "superseded_items": [],
            "terminal_blockers": []
        }
    });

    let normalized = normalize_remediation_inventory(&contract, &routed);

    let items = array(normalized.get("items"));
    assert_eq!(
        items.len(),
        1,
        "only the write failure is remediation work: {}",
        serde_json::to_string_pretty(&normalized).expect("json")
    );
    assert_eq!(items[0]["item_id"], "remediation-allowlist-removal");
    assert!(
        array(normalized.get("unresolved_issues")).is_empty(),
        "a well-formed write failure must not carry issues: {}",
        serde_json::to_string_pretty(&normalized).expect("json")
    );
    assert!(remediation_inventory_ready(&normalized));
}

/// Triage that routed everything to retries has decided no write is needed.
/// That must read as "no work", not as a malformed write item — the router
/// turns a not-ready inventory into `RegenerateInventory`, and a retry batch can
/// never be repaired into a write item, so the two together never converge.
#[test]
fn a_retry_only_triage_inventory_is_empty_work_not_broken_work() {
    let universe = test_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let routed = serde_json::json!({
        "data": {
            "implementation_failures": [],
            "retry_items": [{
                "item_id": "structural-integrity-cargo-batch",
                "canonical_task_ids": ["TASK-X-001"],
                "focused_verification": ["cargo check -p x --tests"]
            }]
        }
    });

    let normalized = normalize_remediation_inventory(&contract, &routed);

    assert!(
        array(normalized.get("items")).is_empty(),
        "a retry batch is not a write item: {}",
        serde_json::to_string_pretty(&normalized).expect("json")
    );
    assert!(
        array(normalized.get("unresolved_issues")).is_empty(),
        "no write item means no write-item issues: {}",
        serde_json::to_string_pretty(&normalized).expect("json")
    );
    assert!(!remediation_inventory_ready(&normalized));
}

/// The classic wave-shaped inventory keeps working exactly as before, and a
/// value with no routed buckets still reaches the generic collector.
#[test]
fn an_items_shaped_inventory_is_still_ready() {
    assert!(remediation_inventory_ready(
        &serde_json::json!({ "items": [{"item_id": "r1"}] })
    ));
    assert!(
        crate::generated_contract::lifecycle_routed_write_items(&serde_json::json!({
            "items": [{"item_id": "r1"}]
        }))
        .is_none(),
        "a non-routed value must not be claimed as a triage inventory"
    );
}

/// Nothing to do is still not ready — this must not become "always ready".
#[test]
fn an_empty_inventory_is_not_ready() {
    for empty in [
        serde_json::json!({}),
        serde_json::json!({"items": []}),
        serde_json::json!({"items": [], "unresolved_issues": []}),
    ] {
        assert!(
            !remediation_inventory_ready(&empty),
            "empty inventory must not be ready: {empty}"
        );
    }
}

/// Unresolved issues still gate a populated inventory.
#[test]
fn unresolved_issues_block_a_populated_inventory() {
    assert!(!remediation_inventory_ready(&serde_json::json!({
        "items": [{"item_id": "r1"}],
        "unresolved_issues": ["x"]
    })));
}
