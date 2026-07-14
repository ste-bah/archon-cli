use super::*;

#[test]
fn every_triage_route_combination_has_a_defined_disposition() {
    for mask in 0u8..8 {
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
                terminal_blockers: Vec::new(),
            };
            let plan = triage_route_plan(&routes);
            let inventory_route = remediation_inventory_route(&plan, inventory_ready);

            assert_eq!(plan.run_retries, mask & 1 != 0, "mask={mask}");
            assert_eq!(plan.try_supersede, mask & 2 != 0, "mask={mask}");
            assert_eq!(plan.run_write_remediation, mask & 4 != 0, "mask={mask}");
            assert!(!plan.terminal_blocked, "mask={mask}");
            match (mask & 4 != 0, inventory_ready) {
                (true, true) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::RunWriteRemediation,
                    "mask={mask}"
                ),
                (true, false) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::RegenerateInventory,
                    "mask={mask}"
                ),
                (false, _) => assert_eq!(
                    inventory_route,
                    RemediationInventoryRoute::NotNeeded,
                    "mask={mask}"
                ),
            }
        }
    }
}

#[test]
fn terminal_blocker_overrides_all_nonterminal_routes() {
    let routes = VerificationTriageRoutes {
        retry_items: vec![serde_json::json!({ "item_id": "retry" })],
        superseded_items: vec![serde_json::json!({ "item_id": "supersede" })],
        implementation_failures: vec![serde_json::json!({ "item_id": "write" })],
        terminal_blockers: vec![serde_json::json!({ "item_id": "terminal" })],
    };
    let plan = triage_route_plan(&routes);

    assert!(plan.terminal_blocked);
    assert_eq!(
        remediation_inventory_route(&plan, true),
        RemediationInventoryRoute::Block
    );
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
        include_str!("fixtures/wfb36_owned_diff_scope_failure.json"),
        include_str!("fixtures/wfb36_owned_diff_scope_retry_failure.json"),
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
    serde_json::from_str(include_str!("fixtures/d31_repeated_gap_retry_chain.json"))
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
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_WAVE_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::RETRY_VERIFICATION_WAVE_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::POST_REMEDIATION_VERIFICATION_WAVE_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::REVIEW_VERIFICATION_WAVE_TASK,
    ];
    for prompt in prompts {
        assert!(prompt.contains("pass_fail_count"));
        assert!(prompt.contains("intended_target_failed"));
        assert!(prompt.contains("matched_test_check_names.failed"));
    }
}

#[test]
fn repair_prompt_requires_write_route_for_reproduced_failures() {
    let prompt = super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_REPAIR_PLAN_TASK;

    assert!(prompt.contains("consistently reproduced"));
    assert!(prompt.contains("route: write_remediation"));
}

#[test]
fn provider_remediation_requires_profile_grounded_redacted_proof() {
    let prompt =
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_REMEDIATION_WAVE_TASK;

    assert!(prompt.contains("source ~/.profile"));
    assert!(prompt.contains("provider_env_proof"));
    assert!(prompt.contains("never credential values"));
}
