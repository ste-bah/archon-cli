use super::super::triage_failed_outcomes;
use super::*;

#[test]
fn every_retry_producer_with_items_routes_to_execution() {
    for producer in [
        RetryProducer::Triage,
        RetryProducer::Retriage,
        RetryProducer::RepairPlan,
    ] {
        assert_eq!(
            retry_consumption_route(producer, &[serde_json::json!({ "item_id": "retry" })]),
            RetryConsumptionRoute::RunRetries,
            "producer={producer:?}"
        );
        assert_eq!(
            retry_consumption_route(producer, &[]),
            RetryConsumptionRoute::NotNeeded,
            "producer={producer:?}"
        );
    }
}

#[test]
fn direct_camel_case_routes_are_canonicalized() {
    let triage = harvest_nested_triage_routes(&serde_json::json!({
        "implementationFailures": [{"item_id": "failed-check"}]
    }));

    assert_eq!(
        triage["implementation_failures"],
        serde_json::json!([{"item_id": "failed-check"}])
    );
    assert_eq!(triage_routes(&triage).implementation_failures.len(), 1);
}

#[test]
fn every_triage_route_combination_has_a_defined_disposition() {
    for mask in 0u8..16 {
        for inventory_ready in [false, true] {
            let routes = VerificationTriageRoutes {
                retry_items: ((mask & 1) != 0)
                    .then(|| serde_json::json!({ "item_id": "retry" }))
                    .into_iter()
                    .collect(),
                superseded_items: ((mask & 2) != 0)
                    .then(|| serde_json::json!({ "item_id": "supersede" }))
                    .into_iter()
                    .collect(),
                implementation_failures: ((mask & 4) != 0)
                    .then(|| serde_json::json!({ "item_id": "write" }))
                    .into_iter()
                    .collect(),
                terminal_blockers: ((mask & 8) != 0)
                    .then(|| {
                        if mask & 1 != 0 {
                            serde_json::json!({
                                "item_id": "terminal",
                                "affected_retry_items": ["retry"]
                            })
                        } else {
                            serde_json::json!({ "item_id": "terminal" })
                        }
                    })
                    .into_iter()
                    .collect(),
            };
            let plan = triage_route_plan(&routes);
            let inventory_route = remediation_inventory_route(&plan, inventory_ready);

            assert_eq!(plan.run_retries, mask & 1 != 0, "mask={mask}");
            assert_eq!(plan.try_supersede, mask & 2 != 0, "mask={mask}");
            assert_eq!(plan.run_write_remediation, mask & 4 != 0, "mask={mask}");
            assert_eq!(
                plan.terminal_blocked,
                mask & 8 != 0 && mask & 1 == 0,
                "mask={mask}"
            );
            match (plan.terminal_blocked, mask & 4 != 0, inventory_ready) {
                (true, _, _) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::Block,
                    "mask={mask}"
                ),
                (false, true, true) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::RunWriteRemediation,
                    "mask={mask}"
                ),
                (false, true, false) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::RegenerateInventory,
                    "mask={mask}"
                ),
                (false, false, _) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::NotNeeded,
                    "mask={mask}"
                ),
            }
        }
    }
}

#[test]
fn coextensive_terminal_blocker_does_not_orphan_retry_work() {
    let routes = VerificationTriageRoutes {
        retry_items: vec![serde_json::json!({ "item_id": "retry" })],
        superseded_items: vec![serde_json::json!({ "item_id": "supersede" })],
        implementation_failures: vec![serde_json::json!({ "item_id": "write" })],
        terminal_blockers: vec![serde_json::json!({
            "item_id": "terminal",
            "affected_retry_items": ["retry"]
        })],
    };
    let plan = triage_route_plan(&routes);

    assert!(!plan.terminal_blocked);
    assert!(plan.run_retries);
    assert!(plan.try_supersede);
    assert!(plan.run_write_remediation);
    assert_eq!(
        remediation_inventory_route(&plan, true),
        RemediationInventoryRoute::RunWriteRemediation
    );
}

#[test]
fn independent_terminal_blocker_remains_fail_closed() {
    let routes = VerificationTriageRoutes {
        retry_items: vec![serde_json::json!({ "item_id": "retry" })],
        terminal_blockers: vec![serde_json::json!({
            "item_id": "external-safety-blocker",
            "affected_retry_items": ["different-item"]
        })],
        ..VerificationTriageRoutes::default()
    };
    let plan = triage_route_plan(&routes);

    assert!(plan.terminal_blocked);
    assert_eq!(
        remediation_inventory_route(&plan, false),
        RemediationInventoryRoute::Block
    );
}

#[test]
fn terminal_blocker_blocks_only_when_no_actionable_route_remains() {
    let routes = VerificationTriageRoutes {
        terminal_blockers: vec![serde_json::json!({ "item_id": "terminal" })],
        ..VerificationTriageRoutes::default()
    };
    let plan = triage_route_plan(&routes);

    assert!(plan.terminal_blocked);
    assert_eq!(
        remediation_inventory_route(&plan, false),
        RemediationInventoryRoute::Block
    );
}

#[test]
fn triage_prompt_forbids_first_sight_retryable_terminal_blockers() {
    let prompt = archon_workflow::v2::lifecycle_prompts::VERIFICATION_FAILURE_TRIAGE_TASK;

    assert!(prompt.contains("Never emit a terminal_blocker"));
    assert!(prompt.contains("at least two retry generations"));
}

#[test]
fn explicit_write_route_selects_named_failed_outcome() {
    let repair = serde_json::json!({
        "status": "accepted",
        "data": {
            "route": "write_remediation",
            "affected_source_outcome_ids": ["failed-artifact-check"]
        }
    });
    let verification = serde_json::json!({ "outcomes": [
        { "item_id": "failed-artifact-check", "status": "failed" },
        { "item_id": "accepted-check", "status": "accepted" }
    ]});

    let selected = write_remediation_outcomes(&repair, &verification);

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0]["item_id"], "failed-artifact-check");
}

#[test]
fn retry_plan_without_write_route_selects_nothing() {
    let repair = serde_json::json!({ "data": { "items": [{ "item_id": "retry" }] } });
    let verification = serde_json::json!({
        "outcomes": [{ "item_id": "failed-check", "status": "failed" }]
    });

    assert!(write_remediation_outcomes(&repair, &verification).is_empty());
}

#[test]
fn unsatisfiable_predicate_route_reauthors_check_with_gap_identity() {
    let fixtures = [
        archon_test_support::fixtures::WFB36_OWNED_DIFF_SCOPE_FAILURE,
        archon_test_support::fixtures::WFB36_OWNED_DIFF_SCOPE_RETRY_FAILURE,
    ];
    for fixture in fixtures {
        let failed: Value = serde_json::from_str(fixture).expect("fixture");
        let source_id = failed["item_id"].as_str().expect("item id");
        let repair = serde_json::json!({ "data": {
            "route": "predicate_unsatisfiable_as_written",
            "re_authored_items": [{
                "item_id": "verify-owned-scope",
                "source_item_id": source_id,
                "focused_verification": "inspect the write manifest"
            }]
        }});
        let verification = serde_json::json!({ "outcomes": [failed] });

        let rewritten = predicate_rewrite_inventory(&repair, &verification)
            .expect("route should produce rewritten inventory");

        assert_eq!(
            rewritten["items"][0]["source_residual_gap_ids"][0],
            "owned-diff-scope"
        );
        assert!(rewritten["items"][0]["failed_predicate"].is_string());
    }
}

fn d31_fixture() -> Value {
    serde_json::from_str(archon_test_support::fixtures::D31_REPEATED_GAP_RETRY_CHAIN)
        .expect("D31 fixture")
}

#[test]
fn one_retry_generation_does_not_escalate() {
    let fixture = d31_fixture();
    let history = fixture["retry_generations"]
        .as_array()
        .expect("retry generations");

    let selected = repeated_gap_write_remediation_outcomes(&history[..1], &history[0]["result"]);

    assert!(selected.is_empty());
}

#[test]
fn second_reproduction_escalates_only_the_matching_gap() {
    let fixture = d31_fixture();
    assert!(
        fixture["repair_plans"]
            .as_array()
            .expect("repair plans")
            .iter()
            .all(|plan| plan["route"] != "write_remediation")
    );
    let history = fixture["retry_generations"]
        .as_array()
        .expect("retry generations");

    let selected = repeated_gap_write_remediation_outcomes(&history[..2], &history[1]["result"]);

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0]["item_id"], "retry-generation-2");
}

#[test]
fn merged_unretried_failure_does_not_count_as_reproduced() {
    let gap = |id: &str, gap_id: &str| {
        serde_json::json!({
            "item_id": id,
            "status": "failed",
            "result": { "residual_gaps": [{ "id": gap_id }] }
        })
    };
    let history = vec![
        serde_json::json!({
            "kind": "verification-retry",
            "result": { "outcomes": [gap("persistent", "persistent-gap")] }
        }),
        serde_json::json!({
            "kind": "verification-triage-retry",
            "verificationPlan": { "items": [{ "item_id": "actual-retry" }] },
            "result": { "outcomes": [
                gap("persistent", "persistent-gap"),
                gap("actual-retry", "different-gap")
            ] }
        }),
    ];

    let selected = repeated_gap_write_remediation_outcomes(&history, &history[1]["result"]);

    assert!(selected.is_empty());
}

#[test]
fn verification_prompts_require_d2_failure_fields() {
    let prompts = [
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::RETRY_VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::POST_REMEDIATION_VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::REVIEW_VERIFICATION_WAVE_TASK,
    ];
    for prompt in prompts {
        assert!(prompt.contains("pass_fail_count"));
        assert!(prompt.contains("intended_target_failed"));
        assert!(prompt.contains("matched_test_check_names.failed"));
    }
}

#[test]
fn repair_prompt_requires_write_route_for_reproduced_failures() {
    let prompt = archon_workflow::v2::lifecycle_prompts::VERIFICATION_REPAIR_PLAN_TASK;

    assert!(prompt.contains("consistently reproduced"));
    assert!(prompt.contains("route: write_remediation"));
}

#[test]
fn provider_remediation_requires_host_grounded_redacted_proof() {
    let prompt = archon_workflow::v2::lifecycle_prompts::VERIFICATION_REMEDIATION_WAVE_TASK;

    assert!(prompt.contains("host-injected run-scoped provider environment"));
    assert!(prompt.contains("provider_env_proof"));
    assert!(prompt.contains("never credential values"));
}

#[test]
fn d45_keyless_and_or_unavailable_contracts_are_accepted_at_source() {
    let prompts = [
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_PLAN_TASK,
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_REMEDIATION_WAVE_TASK,
    ];

    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("checked_keys=[]"))
    );
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("OR unavailable"))
    );
}

#[test]
fn d48_d49_verifier_prompts_search_branch_proofs_and_follow_manifest_pointer() {
    let prompts = [
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_PLAN_TASK,
        archon_workflow::v2::lifecycle_prompts::VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::RETRY_VERIFICATION_WAVE_TASK,
        archon_workflow::v2::lifecycle_prompts::FINAL_EVIDENCE_RECONCILIATION_TASK,
        archon_workflow::v2::lifecycle_prompts::FINAL_ZERO_GAP_AUDIT_TASK,
    ];

    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("manifest.normalized_path"))
    );
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("workflow_branch_evidence_root"))
    );
}

#[test]
fn d69_routes_nested_under_items_container_are_harvested() {
    // wf-98c76722 fixture shape: the reducer authored a valid actionable item
    // but nested it under data.items.implementation_failures.
    let triage = serde_json::json!({
        "data": {
            "items": {
                "implementation_failures": [{
                    "item_id": "triage-impl-TASK-EX-001-stale-evidence",
                    "canonical_task_ids": ["TASK-EX-001"],
                    "classification": "actionable_implementation_failure",
                }]
            }
        }
    });
    let routes = triage_routes(&triage);
    assert!(
        routes.implementation_failures.is_empty(),
        "raw read is empty"
    );
    let harvested = harvest_nested_triage_routes(&triage);
    let routes = triage_routes(&harvested);
    assert_eq!(routes.implementation_failures.len(), 1);
}

#[test]
fn d69_nested_camel_case_routes_are_harvested_and_deduped() {
    let item = serde_json::json!({ "item_id": "retry-check", "classification": "retryable_verification_shape_issue" });
    let triage = serde_json::json!({
        "result": { "data": {
            "retry_items": [item],
            "triage": { "retryItems": [item] }
        }}
    });
    let harvested = harvest_nested_triage_routes(&triage);
    let routes = triage_routes(&harvested);
    assert_eq!(routes.retry_items.len(), 1, "hoisted duplicate is deduped");
}

#[test]
fn d69_unaccounted_failed_outcomes_require_shape_repair() {
    let failed =
        [serde_json::json!({ "item_id": "verify-TASK-EX-001-evidence", "status": "failed" })];
    let empty_triage = serde_json::json!({ "data": {
        "retry_items": [], "implementation_failures": [],
        "superseded_items": [], "terminal_blockers": []
    }});
    assert_eq!(unaccounted_failed_outcomes(&empty_triage, &failed).len(), 1);
    let accounted = serde_json::json!({ "data": {
        "implementation_failures": [{
            "item_id": "triage-impl-stale",
            "source_item_id": "verify-TASK-EX-001-evidence",
        }]
    }});
    assert!(unaccounted_failed_outcomes(&accounted, &failed).is_empty());
}

#[test]
fn d71_post_remediation_envelope_supplies_non_vacuous_triage_denominator() {
    let fixture: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::D71_POST_REMEDIATION_TRIAGE_ENVELOPE)
            .expect("D71 fixture");
    let failed = triage_failed_outcomes(&fixture["verification"]);

    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["item_id"], "verify-TASK-EX-001-artifact");
    let harvested = harvest_nested_triage_routes(&fixture["canary_triage"]);
    assert!(unaccounted_failed_outcomes(&harvested, &failed).is_empty());

    let empty_routes = serde_json::json!({"data": {
        "implementation_failures": [], "retry_items": [],
        "superseded_items": [], "terminal_blockers": []
    }});
    assert_eq!(unaccounted_failed_outcomes(&empty_routes, &failed).len(), 1);
}

#[test]
fn d71_nonaccepted_empty_envelope_surfaces_wiring_failure() {
    let failed = triage_failed_outcomes(&serde_json::json!({
        "status": "needs_review",
        "summary": "merged verification lost branch outcomes"
    }));

    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["failure_kind"], "triage_denominator_wiring_error");
}
