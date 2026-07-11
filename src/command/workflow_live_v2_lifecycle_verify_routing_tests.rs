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
