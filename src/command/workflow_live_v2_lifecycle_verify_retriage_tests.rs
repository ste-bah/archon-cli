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

