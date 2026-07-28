#[test]
fn transport_failed_inventory_reducer_is_retried_before_terminal() {
    let failed = serde_json::json!({
        "status": "failed",
        "summary": "agent transport failed: reactive subagent compaction failed: no safe compaction boundary",
        "data": { "error": "no safe compaction boundary" }
    });

    assert_eq!(
        inventory_transport_route(&failed, 1, 2),
        InventoryTransportRoute::Retry
    );
    assert_eq!(
        inventory_transport_route(&failed, 2, 2),
        InventoryTransportRoute::Exhausted(
            "agent transport failed: reactive subagent compaction failed: no safe compaction boundary"
                .to_string()
        )
    );

    let recovered = serde_json::json!({
        "status": "accepted",
        "summary": "fresh reducer returned a remediation inventory",
        "items": [{"item_id": "repair-one"}]
    });
    assert_eq!(
        inventory_transport_route(&recovered, 2, 2),
        InventoryTransportRoute::UseResult
    );

    let mut attempts = Vec::new();
    support::record_repair_attempt(
        &mut attempts,
        "verification-remediation-inventory-1-1-regenerate-2",
        "verification_remediation_inventory",
        &[],
        &failed,
    );
    assert_eq!(attempts[0]["reason"], failed["summary"]);
}

#[test]
fn semantic_empty_inventory_is_not_retried_as_transport() {
    let empty = serde_json::json!({
        "status": "needs_review",
        "summary": "no actionable implementation failures",
        "items": []
    });

    assert_eq!(
        inventory_transport_route(&empty, 1, 2),
        InventoryTransportRoute::UseResult
    );
}

#[test]
fn transport_error_before_json_is_promoted_to_the_same_retry_route() {
    let failed = transport_failure_result(
        "verification-remediation-inventory-4-1",
        1,
        "reactive subagent compaction failed: no safe compaction boundary",
    );

    assert_eq!(
        inventory_transport_route(&failed, 1, 2),
        InventoryTransportRoute::Retry
    );
    assert!(
        failed["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("no safe compaction boundary"))
    );
}

#[test]
fn exhausted_transport_is_an_explicit_infrastructure_blocker() {
    let failed = transport_failure_result(
        "verification-failure-triage-7-1",
        2,
        "reactive subagent compaction failed: no safe compaction boundary",
    );

    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["data"]["failure_class"], "transport_infrastructure");
    assert_eq!(failed["data"]["transport_exhausted"], true);
    assert_eq!(failed["data"]["transport_attempts"], 2);
    assert_eq!(
        failed["data"]["terminal_blockers"][0]["classification"],
        "transport_infrastructure_exhausted"
    );
    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&failed);
    let plan = workflow_live_v2_lifecycle_verify_routing::triage_route_plan(&routes);
    assert!(routes.implementation_failures.is_empty());
    assert!(routes.retry_items.is_empty());
    assert_eq!(routes.terminal_blockers.len(), 1);
    assert!(plan.terminal_blocked);
}

#[test]
fn every_lifecycle_reducer_uses_the_common_transport_retry_path() {
    let lifecycle = include_str!("workflow_live_v2_lifecycle.rs");
    let reducer_call_sites = [
        "workflow_live_v2_lifecycle_impl.rs",
        "workflow_live_v2_lifecycle_review.rs",
        "workflow_live_v2_lifecycle_review_verification.rs",
        "workflow_live_v2_lifecycle_verify.rs",
        "workflow_live_v2_lifecycle_verify_outcome_repair_driver.rs",
        "workflow_live_v2_lifecycle_verify_remediation.rs",
        "workflow_live_v2_lifecycle_verify_triage.rs",
        "workflow_live_v2_lifecycle_waves.rs",
    ];

    assert!(lifecycle.contains("for attempt in 1..=max_transport_attempts"));
    assert!(lifecycle.contains("transport_failure_summary(&result)"));
    assert!(lifecycle.contains("source_pack_value(&source)"));
    for source in reducer_call_sites {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/command")
                .join(source),
        )
        .expect("read lifecycle reducer source");
        assert!(
            !text.contains(".call(\"reduce\""),
            "{source} bypasses LifecycleDriver::reduce"
        );
    }
}

#[test]
fn slim_inventory_input_collapses_check_clones_and_records_count() {
    let clones = (1..=10)
        .map(|index| {
            serde_json::json!({
                "item_id": format!("verify-file-size-check-{index}"),
                "status": "failed",
                "result": {
                    "status": "failed",
                    "summary": "same focused predicate failed",
                    "residual_gaps": [{"id": "gap-file-size", "description": "must pass"}]
                }
            })
        })
        .collect::<Vec<_>>();

    let slim = slim_verification_records(
        &[serde_json::json!({
            "kind": "verification-retry",
            "result": { "status": "failed", "outcomes": clones }
        })],
        false,
    );
    let outcomes = slim[0]["result"]["outcomes"].as_array().unwrap();

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0]["duplicate_count"], 10);
    assert_eq!(outcomes[0]["item_id"], "verify-file-size-check-1");
}

#[test]
fn slim_inventory_input_preserves_distinct_gaps_under_one_stem() {
    let outcomes = ["gap-a", "gap-b"]
        .into_iter()
        .enumerate()
        .map(|(index, gap_id)| {
            serde_json::json!({
                "item_id": format!("verify-source-check-{}", index + 1),
                "status": "failed",
                "result": {
                    "status": "failed",
                    "residual_gaps": [{"id": gap_id, "description": "distinct predicate"}]
                }
            })
        })
        .collect::<Vec<_>>();

    let (collapsed, omitted) = collapse_outcome_clones(&outcomes, 8);

    assert_eq!(collapsed.len(), 2);
    assert_eq!(omitted, 0);
    assert!(
        collapsed
            .iter()
            .all(|outcome| outcome["duplicate_count"] == 1)
    );
}

#[test]
fn oversized_verification_reducers_use_distinct_slim_retry_payloads() {
    let items = (0..80)
        .map(|index| {
            serde_json::json!({
                "item_id": format!("verify-{index}"),
                "evidence_blob": "x".repeat(1024)
            })
        })
        .collect::<Vec<_>>();
    let records = (0..12)
        .map(|index| {
            serde_json::json!({
                "kind": "verification",
                "result": {
                    "status": "failed",
                    "outcomes": items,
                    "summary": format!("attempt {index}")
                }
            })
        })
        .collect::<Vec<_>>();
    let sources = [
        (
            "verification-failure-triage-1-1",
            serde_json::json!([{}, items, items, items, records, records]),
        ),
        (
            "verification-failure-retriage-1-1",
            serde_json::json!([{}, items, items, {
                "failed_outcome_ids": items,
                "failed_outcomes": items,
                "rejected_triage": {"outcomes": items}
            }]),
        ),
        (
            "verification-repair-plan-1-1",
            serde_json::json!([{}, items, {"outcomes": items}, records]),
        ),
    ];

    for (id, source) in sources {
        let normal = slim_reducer_source(id, &source, false);
        let retry = slim_reducer_source(id, &source, true);
        assert!(
            serde_json::to_vec(&normal).unwrap().len() < serde_json::to_vec(&source).unwrap().len()
        );
        assert!(
            serde_json::to_vec(&retry).unwrap().len() < serde_json::to_vec(&normal).unwrap().len()
        );
    }
}

#[test]
fn final_report_input_normalizes_nested_null_collections_only() {
    let mut input = serde_json::json!({
        "triage": {
            "outcomes": null,
            "retry_items": null,
            "data": null
        },
        "task_coverage": null
    });

    normalize_null_report_collections(&mut input);

    assert_eq!(input["triage"]["outcomes"], serde_json::json!([]));
    assert_eq!(input["triage"]["retry_items"], serde_json::json!([]));
    assert_eq!(input["task_coverage"], serde_json::json!([]));
    assert!(input["triage"]["data"].is_null());
}

#[test]
fn failed_final_report_terminal_marker_routes_to_host_fallback() {
    assert!(terminal_marker_requires_report_fallback(Some(
        WorkflowV2Status::Failed
    )));
    assert!(!terminal_marker_requires_report_fallback(Some(
        WorkflowV2Status::NeedsReview
    )));
    assert!(!terminal_marker_requires_report_fallback(None));
}
