use super::*;

pub(crate) fn contract_fixture() -> (WorkflowV2TaskUniverse, serde_json::Value) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-010".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-TDL-010.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    };
    let plan_item = serde_json::json!({
        "item_id": "verify-plan",
        "canonical_task_ids": ["TASK-TDL-010"],
        "focused_verification": ["check"],
        "expected_evidence": ["evidence"],
    });
    (universe, plan_item)
}

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
        2,
        "reactive subagent compaction failed: no safe compaction boundary",
    );

    assert_eq!(
        inventory_transport_route(&failed, 1, 2),
        InventoryTransportRoute::Retry
    );
    assert_eq!(failed["data"]["transport_exhausted"], false);
    assert_eq!(failed["data"]["max_transport_attempts"], 2);
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
        2,
        "reactive subagent compaction failed: no safe compaction boundary",
    );

    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["data"]["failure_class"], "transport_infrastructure");
    assert_eq!(failed["data"]["transport_exhausted"], true);
    assert_eq!(failed["data"]["transport_attempts"], 2);
    assert_eq!(failed["data"]["max_transport_attempts"], 2);
    assert_eq!(
        failed["data"]["terminal_blockers"][0]["classification"],
        "transport_infrastructure_exhausted"
    );
    let routes = lifecycle_policy::verify_routing::triage_routes(&failed);
    let plan = lifecycle_policy::verify_routing::triage_route_plan(&routes);
    assert!(routes.implementation_failures.is_empty());
    assert!(routes.retry_items.is_empty());
    assert_eq!(routes.terminal_blockers.len(), 1);
    assert!(plan.terminal_blocked);
}

#[test]
fn every_lifecycle_reducer_uses_the_common_transport_retry_path() {
    // The driver's method bodies live in the sibling parts since the
    // file-size split; the contract is module-wide, not file-wide.
    let lifecycle = [
        include_str!("mod.rs"),
        include_str!("driver_a.rs"),
        include_str!("driver_b.rs"),
        include_str!("driver_c.rs"),
    ]
    .concat();
    let reducer_call_sites = [
        "implementation.rs",
        "review.rs",
        "review_verification.rs",
        "verify.rs",
        "verify_outcome_repair.rs",
        "verify_remediation.rs",
        "verify_triage.rs",
        "waves.rs",
    ];

    assert!(lifecycle.contains("for attempt in 1..=max_transport_attempts"));
    assert!(lifecycle.contains("transport_failure_summary(&result)"));
    // The transport retry still packs the source; it now asks the host to do
    // it through the lifecycle host port rather than naming the binary's
    // `source_pack_value` directly.
    assert!(lifecycle.contains("pack_reduce_source(&source)"));
    for source in reducer_call_sites {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/v2/lifecycle_driver")
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

#[test]
fn mixed_triage_preserves_actionable_and_retry_routes() {
    let triage: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::WF3B9_VERIFICATION_FAILURE_TRIAGE_5_3)
            .expect("D25 fixture");

    let routes = lifecycle_policy::verify_routing::triage_routes(&triage);

    assert_eq!(routes.implementation_failures.len(), 1);
    assert_eq!(routes.retry_items.len(), 3);
    assert_eq!(
        routes.implementation_failures[0]["classification"],
        "actionable_implementation_failure"
    );
}

#[test]
fn d46_execution_retry_is_scheduled_even_when_write_inventory_is_empty() {
    let fixture: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::D46_ORPHANED_VERIFICATION_RETRY)
            .expect("D46 fixture");
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-060".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-TDL-060.md".to_string(),
            dependency_ids: Vec::new(),
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        }],
    };
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let failed = vec![fixture["failed_outcome"].clone()];

    let retries = producer_retry_items(
        &contract,
        &fixture["triage"],
        lifecycle_policy::verify_routing::RetryProducer::Triage,
        &[fixture["plan_item"].clone()],
        &failed,
    )
    .expect("D46 execution retry must remain schedulable");
    let routes = lifecycle_policy::verify_routing::triage_routes(&fixture["triage"]);
    let plan = lifecycle_policy::verify_routing::triage_route_plan(&routes);

    assert_eq!(retries.len(), 1);
    assert_eq!(
        retries[0]["classification"],
        "retry_resolved_verification_execution_issue"
    );
    assert!(plan.run_retries);
    assert!(plan.try_supersede);
    assert!(!plan.run_write_remediation);
    assert_eq!(
        lifecycle_policy::verify_routing::remediation_inventory_route(&plan, false),
        lifecycle_policy::verify_routing::RemediationInventoryRoute::NotNeeded
    );
}

#[test]
fn mixed_supersede_marks_only_its_failed_outcome() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            accepted_outcome("accepted-sibling"),
            failed_outcome_with_gap("failed-shape", "shape-gap"),
            failed_outcome_with_gap("failed-implementation", "implementation-gap")
        ]
    });
    let triage = serde_json::json!({ "data": {
        "superseded_items": [{
            "item_id": "supersede-shape",
            "source_item_id": "failed-shape",
            "canonical_task_ids": ["TASK-TDL-010"],
            "classification": "retry_resolved_by_sibling_evidence",
            "source_residual_gap_ids": ["shape-gap"],
            "failed_predicate": "shape-gap"
        }],
        "implementation_failures": [{
            "item_id": "repair-implementation",
            "source_item_id": "failed-implementation",
            "canonical_task_ids": ["TASK-TDL-010"],
            "classification": "actionable_implementation_failure"
        }]
    }});

    let result = lifecycle_policy::verify_supersede::try_supersede_verification(
        &contract,
        &verification,
        &triage,
        "triage-mixed",
    )
    .expect("the selected shape failure is provably supersedable");
    let outcomes = support::outcomes_of(&result.verification);

    assert_eq!(result.verification["status"], "needs_review");
    assert!(
        outcomes
            .iter()
            .any(|outcome| { outcome["item_id"] == "failed-shape" && outcome["status"] == "noop" })
    );
    assert!(outcomes.iter().any(|outcome| {
        outcome["item_id"] == "failed-implementation" && outcome["status"] == "failed"
    }));
}

#[test]
fn fabel_triage_retry_items_keep_only_required_reruns() {
    let (universe, plan_item) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let triage: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::WFFED_VERIFICATION_FAILURE_TRIAGE_1_2)
            .expect("fixture json");

    let source_outcomes = vec![failed_outcome_with_gap(
        "verification-wave-1-1-VERIFY-TDL-010-003-project-registry-artifact-contract-check-7",
        "gap-healthy-dataset-required-artifact-path-fields",
    )];
    let retry_items = producer_retry_items(
        &contract,
        &triage,
        lifecycle_policy::verify_routing::RetryProducer::Triage,
        &[plan_item],
        &source_outcomes,
    );

    assert!(
        retry_items.is_none(),
        "legacy triage fixture retains unrelated contract defects"
    );
}
