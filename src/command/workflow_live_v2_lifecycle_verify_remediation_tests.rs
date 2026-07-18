use super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask;
use super::*;

fn contract_fixture() -> (WorkflowV2TaskUniverse, serde_json::Value) {
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

#[test]
fn mixed_triage_preserves_actionable_and_retry_routes() {
    let triage: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf3b9_verification_failure_triage_5_3.json"
    ))
    .expect("D25 fixture");

    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&triage);

    assert_eq!(routes.implementation_failures.len(), 1);
    assert_eq!(routes.retry_items.len(), 3);
    assert_eq!(
        routes.implementation_failures[0]["classification"],
        "actionable_implementation_failure"
    );
}

#[test]
fn d46_execution_retry_is_scheduled_even_when_write_inventory_is_empty() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/d46_orphaned_verification_retry.json"
    ))
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
        workflow_live_v2_lifecycle_verify_routing::RetryProducer::Triage,
        &[fixture["plan_item"].clone()],
        &failed,
    )
    .expect("D46 execution retry must remain schedulable");
    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&fixture["triage"]);
    let plan = workflow_live_v2_lifecycle_verify_routing::triage_route_plan(&routes);

    assert_eq!(retries.len(), 1);
    assert_eq!(
        retries[0]["classification"],
        "retry_resolved_verification_execution_issue"
    );
    assert!(plan.run_retries);
    assert!(plan.try_supersede);
    assert!(!plan.run_write_remediation);
    assert_eq!(
        workflow_live_v2_lifecycle_verify_routing::remediation_inventory_route(&plan, false),
        workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::NotNeeded
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

    let result = workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
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
    let triage: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wffed_verification_failure_triage_1_2.json"
    ))
    .expect("fixture json");

    let source_outcomes = vec![failed_outcome_with_gap(
        "verification-wave-1-1-VERIFY-TDL-010-003-project-registry-artifact-contract-check-7",
        "gap-healthy-dataset-required-artifact-path-fields",
    )];
    let retry_items = producer_retry_items(
        &contract,
        &triage,
        workflow_live_v2_lifecycle_verify_routing::RetryProducer::Triage,
        &[plan_item],
        &source_outcomes,
    );

    assert!(
        retry_items.is_none(),
        "legacy triage fixture retains unrelated contract defects"
    );
}

#[test]
fn fabel_supersede_accepts_shape_failure_with_sibling_evidence() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            accepted_outcome("accepted-sibling"),
            failed_outcome_with_gap("failed-shape", "shape-gap")
        ]
    });
    let triage = serde_json::json!({
        "status": "accepted",
        "data": {
            "implementation_failures": [],
            "terminal_blockers": [],
            "retry_items": [{
                "item_id": "failed-shape",
                "classification": "retryable_verification_shape_issue",
                "source_residual_gap_ids": ["shape-gap"],
                "failed_predicate": "failed invariant"
            }]
        }
    });

    let supersede = workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
        &contract,
        &verification,
        &triage,
        "verification-failure-triage",
    )
    .expect("supersede");

    assert_eq!(supersede.verification["status"], "accepted");
    assert_eq!(
        supersede.record["superseded"][0]["failed_outcome_id"],
        "failed-shape"
    );
}

#[test]
fn supersede_rejects_sibling_evidence_for_a_different_invariant() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            accepted_outcome("accepted-sibling"),
            failed_outcome_with_gap("failed-shape", "provider-env-mismatch")
        ]
    });
    let triage = serde_json::json!({
        "status": "accepted",
        "data": {
            "implementation_failures": [],
            "terminal_blockers": [],
            "retry_items": [{
                "item_id": "failed-shape",
                "classification": "retryable_verification_shape_issue",
                "source_residual_gap_ids": ["evidence-shape-gap"],
                "failed_predicate": "evidence envelope must be valid"
            }]
        }
    });

    assert!(
        workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
            &contract,
            &verification,
            &triage,
            "verification-failure-triage",
        )
        .is_none()
    );
}

#[test]
fn d32_unprovable_supersede_requires_one_bounded_retriage() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/d32_zero_match_retriage.json"))
            .expect("D32 fixture");

    assert!(
        workflow_live_v2_lifecycle_verify_retriage::needs_bounded_retriage(
            &contract,
            &fixture["verification"],
            &fixture["triage"],
        )
    );
    assert!(
        workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
            &contract,
            &fixture["verification"],
            &fixture["triage"],
            "verification-failure-triage-4-1",
        )
        .is_none()
    );
    let feedback = workflow_live_v2_lifecycle_verify_retriage::retriage_feedback(
        &fixture["verification"],
        &fixture["triage"],
    );
    assert_eq!(
        support::strings_of(feedback.get("failed_outcome_ids")).len(),
        4
    );
    assert_eq!(feedback["required_route"], "corrected_retry_items");
}

#[test]
fn d32_triage_prompt_routes_stale_filters_to_corrected_retries() {
    let prompt = workflow_live_v2_lifecycle_prompts::VERIFICATION_FAILURE_TRIAGE_TASK;

    assert!(prompt.contains("zero-match"));
    assert!(prompt.contains("repository-search-verified"));
    assert!(prompt.contains("never superseded_items"));
    assert!(prompt.contains("corrected exact test names"));
}

#[test]
fn d32_corrected_retries_validate_against_every_failed_outcome() {
    let (universe, plan_item) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/d32_zero_match_retriage.json"))
            .expect("D32 fixture");
    let failed = support::non_accepted_outcomes(&support::outcomes_of(&fixture["verification"]));
    let retry_items: Vec<serde_json::Value> = failed.iter().map(corrected_retry_item).collect();
    let triage = serde_json::json!({ "data": { "retry_items": retry_items } });

    let retries = producer_retry_items(
        &contract,
        &triage,
        workflow_live_v2_lifecycle_verify_routing::RetryProducer::Retriage,
        &[plan_item],
        &failed,
    )
    .expect("all corrected retries should validate");

    assert_eq!(retries.len(), 4);
    assert!(
        !workflow_live_v2_lifecycle_verify_retriage::needs_bounded_retriage(
            &contract,
            &fixture["verification"],
            &triage,
        )
    );
}

#[test]
fn d63_retriage_retries_survive_generated_outcome_ids_and_remain_distinct() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/d63_retriage_retry_consumer.json"))
            .expect("D63 fixture");
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-080".to_string(),
            aliases: Vec::new(),
            source_path: "tasks/TASK-TDL-080.md".to_string(),
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
    let plan_items = support::array(fixture.get("plan_items"));
    let source_outcomes = support::array(fixture.get("source_outcomes"));

    let retries = producer_retry_items(
        &contract,
        &fixture["retriage"],
        workflow_live_v2_lifecycle_verify_routing::RetryProducer::Retriage,
        &plan_items,
        &source_outcomes,
    )
    .expect("retriage retry items must be schedulable");

    assert_eq!(retries.len(), 2);
    assert_ne!(retries[0]["item_id"], retries[1]["item_id"]);
    assert_eq!(
        workflow_live_v2_lifecycle_verify_routing::retry_consumption_route(
            workflow_live_v2_lifecycle_verify_routing::RetryProducer::Retriage,
            &retries,
        ),
        workflow_live_v2_lifecycle_verify_routing::RetryConsumptionRoute::RunRetries
    );
    assert_eq!(
        fixture["terminal_call"], "blocked-verification-failed-2",
        "fixture documents the pre-D63 terminal that must follow retry execution now"
    );
}

#[test]
fn retry_inventory_stamps_a_dropped_source_gap() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture");

    let inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["invalid_retry_plan"],
        &fixture["initial_verification"],
    );

    assert!(support::array(inventory.get("unresolved_issues")).is_empty());
    assert_eq!(
        inventory["items"][0]["source_residual_gap_ids"],
        fixture["valid_retry_plan"]["items"][0]["source_residual_gap_ids"]
    );
}

#[test]
fn canary_shape_repair_rounds_use_stable_host_gap_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf6dd_verification_retry_invariant_failure.json"
    ))
    .expect("fixture JSON");
    for key in ["round_1_inventory", "round_3_inventory"] {
        let checked = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
            &fixture[key],
            &fixture["verification"],
        );
        assert!(support::array(checked.get("unresolved_issues")).is_empty());
        assert_eq!(
            checked["items"][0]["source_residual_gap_ids"],
            serde_json::json!(["manifest-required-evidence-missing"])
        );
        assert_eq!(
            checked["items"][0]["failed_predicate"],
            "Manifest must include registry and focused command evidence."
        );
    }
}

#[test]
fn retry_inventory_without_matching_failure_is_rejected() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf6dd_verification_retry_invariant_failure.json"
    ))
    .expect("fixture JSON");
    let checked = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["unmatched_inventory"],
        &fixture["verification"],
    );

    assert!(
        support::array(checked.get("unresolved_issues"))
            .iter()
            .any(|issue| issue["field"] == "source_item_id")
    );
}

#[test]
fn retry_inventory_accepts_the_exact_source_gap_and_predicate() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture");

    let inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &fixture["valid_retry_plan"],
        &fixture["initial_verification"],
    );

    assert!(support::array(inventory.get("unresolved_issues")).is_empty());
}

#[test]
fn fabel_shape_repair_drops_already_accepted_source_outcomes() {
    let (universe, _) = contract_fixture();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let verification = serde_json::json!({
        "outcomes": [accepted_outcome("accepted-source"), failed_outcome("failed-source")]
    });
    let inventory = serde_json::json!({
        "items": [
            retry_item("retry-accepted", "accepted-source"),
            retry_item("retry-failed", "failed-source")
        ]
    });

    let scoped = scope_repair_inventory_to_failed_outcomes(&contract, &inventory, &verification);
    let items = support::array(scoped.get("items"));

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["item_id"], "retry-failed");
}

#[test]
fn d22_verification_remediation_source_items_satisfy_graph_contract() {
    let item: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf485_verification_remediation_source_item.json"
    ))
    .expect("D22 fixture");
    let source_items =
        workflow_live_v2_lifecycle_verify_merge::verification_remediation_source_items(
            &serde_json::json!({ "items": [item] }),
        );

    assert_eq!(
        source_items[0]["verification_requirements"],
        source_items[0]["focused_verification"]
    );
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-050".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-TDL-050.md".to_string(),
                dependency_ids: vec!["TASK-TDL-030".to_string()],
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-030".to_string(),
                aliases: Vec::new(),
                source_path: "tasks/TASK-TDL-030.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
        ],
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-5-verification-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "source_data": source_items }),
        depends_on: Vec::new(),
    };
    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);
    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
}

#[test]
fn d23_retry_merge_preserves_unretried_failures() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wf485_verification_retry_merge.json"))
            .expect("D23 fixture");
    let retry_items = support::array(fixture.get("retry_items"));
    let merged = workflow_live_v2_lifecycle_verify_merge::merge_retry_outcomes(
        &fixture["initial"],
        fixture["retry_result"].clone(),
        &retry_items,
    );
    let outcomes = support::outcomes_of(&merged);

    assert_eq!(outcomes.len(), 9);
    assert_eq!(merged["status"], "needs_review");
    assert_eq!(
        merged["summary"],
        "verification retry merged 9 outcomes with 2 unresolved"
    );
    assert!(outcomes.iter().any(|outcome| {
        outcome["item_id"] == "verify-040-artifact" && outcome["status"] == "failed"
    }));
    assert!(
        !outcomes
            .iter()
            .any(|outcome| outcome["item_id"] == "verify-050-artifact")
    );
    assert!(outcomes.iter().any(|outcome| {
        outcome["source_item_id"] == "verify-050-artifact" && outcome["status"] == "accepted"
    }));
}

fn accepted_outcome(id: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "status": "accepted",
        "canonical_task_ids": ["TASK-TDL-010"],
        "evidence": [{ "summary": "shape-gap resolved: evidence envelope must be valid" }],
        "resolved_residual_gap_ids": ["shape-gap"]
    })
}

fn failed_outcome(id: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "status": "failed",
        "canonical_task_ids": ["TASK-TDL-010"],
        "result": { "data": { "verification_failure_class": "retryable_verification_issue" } }
    })
}

fn failed_outcome_with_gap(id: &str, gap_id: &str) -> serde_json::Value {
    let mut outcome = failed_outcome(id);
    outcome["result"]["residual_gaps"] = serde_json::json!([{
        "id": gap_id,
        "description": "failed invariant",
        "severity": "review"
    }]);
    outcome
}

fn retry_item(id: &str, source: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "source_outcome_item_ids": [source],
        "canonical_task_ids": ["TASK-TDL-010"],
        "focused_verification": ["retry check"],
        "expected_evidence": ["retry evidence"]
    })
}

fn corrected_retry_item(outcome: &serde_json::Value) -> serde_json::Value {
    let item_id = outcome["item_id"].as_str().expect("item id");
    let gap = &outcome["result"]["residual_gaps"][0];
    serde_json::json!({
        "item_id": item_id,
        "source_item_id": item_id,
        "canonical_task_ids": ["TASK-TDL-010"],
        "classification": "retryable_verification_shape_issue",
        "source_residual_gap_ids": [gap["id"].clone()],
        "failed_predicate": gap["description"].clone(),
        "focused_verification": ["cargo test trading_data_validation_and_provider_commands_parse"],
        "expected_evidence": ["The corrected exact test name runs and passes."]
    })
}
