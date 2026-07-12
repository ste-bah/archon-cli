use super::*;

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
