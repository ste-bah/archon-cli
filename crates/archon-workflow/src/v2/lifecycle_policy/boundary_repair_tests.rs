use super::*;

#[test]
fn outcome_repair_harvests_only_known_nested_collections() {
    let raw = serde_json::json!({
        "data": {
            "items": {
                "remediationItems": [{"item_id": "repair-a", "source_item_id": "failed-a"}]
            },
            "untrusted": {
                "items": [{"item_id": "repair-b", "source_item_id": "failed-b"}]
            }
        }
    });

    let harvested = harvest_outcome_repair_items(&raw);

    assert_eq!(
        collection_items(&harvested),
        vec![serde_json::json!({
            "item_id": "repair-a",
            "source_item_id": "failed-a"
        })]
    );
}

#[test]
fn outcome_repair_quality_requires_every_failed_outcome_to_be_accounted() {
    let failed = vec![
        serde_json::json!({"item_id": "failed-a"}),
        serde_json::json!({"item_id": "failed-b"}),
    ];
    let partial = serde_json::json!({
        "items": [{"item_id": "repair-a", "source_item_id": "failed-a"}],
        "unresolved_issues": []
    });
    let complete = serde_json::json!({
        "items": [
            {"item_id": "repair-a", "source_item_id": "failed-a"},
            {"item_id": "repair-b", "source_item_id": "failed-b"}
        ],
        "unresolved_issues": []
    });

    assert!(outcome_repair_quality(&complete, &failed) < outcome_repair_quality(&partial, &failed));
    assert_eq!(outcome_repair_quality(&complete, &failed).unaccounted, 0);
}

#[test]
fn reconciliation_harvest_and_validity_reject_malformed_items() {
    let nested = harvest_reconciliation_items(&serde_json::json!({
        "result": {
            "data": {
                "reconciliation": {
                    "evidenceIssues": [{"id": "issue-a"}, {"id": "issue-a"}]
                }
            }
        }
    }));
    assert_eq!(collection_items(&nested).len(), 1);
    assert_eq!(
        reconciliation_quality(&nested),
        ReconciliationQuality {
            missing_collection: 0,
            malformed_items: 0,
        }
    );

    let malformed = harvest_reconciliation_items(&serde_json::json!({"items": ["issue-a"]}));
    assert_eq!(reconciliation_quality(&malformed).malformed_items, 1);
    assert_eq!(
        reconciliation_quality(&malformed).defect_count(),
        reconciliation_quality(&serde_json::json!({})).defect_count()
    );
}
