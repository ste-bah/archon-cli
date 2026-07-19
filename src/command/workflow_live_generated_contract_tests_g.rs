fn invariant_chain_fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/wf32_verification_invariant_chain.json"
    ))
    .expect("D17 fixture")
}

#[test]
fn retry_item_dropping_source_invariant_is_repairable_shape_issue() {
    let fixture = invariant_chain_fixture();
    let inventory = normalize_generated_inventory_value(
        &fixture["invalid_retry_plan"],
        None,
    );

    assert!(inventory.issues.iter().any(|issue| {
        issue.kind == GeneratedContractIssueKind::EvidenceRepair
            && issue.field == "source_residual_gap_ids"
    }));
    assert!(inventory.issues.iter().any(|issue| {
        issue.kind == GeneratedContractIssueKind::EvidenceRepair
            && issue.field == "failed_predicate"
    }));
}

#[test]
fn retry_item_preserving_source_invariant_is_contract_valid() {
    let fixture = invariant_chain_fixture();
    let inventory = normalize_generated_inventory_value(
        &fixture["valid_retry_plan"],
        None,
    );

    assert!(inventory.issues.is_empty(), "{:?}", inventory.issues);
    assert_eq!(
        inventory.items[0]["source_residual_gap_ids"][0],
        "provider-env-status-mismatch"
    );
    assert_eq!(
        inventory.items[0]["failed_predicate"],
        "Artifact credential status must match current redacted provider proof."
    );
}

#[test]
fn retry_invariant_aliases_normalize_to_canonical_fields() {
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "retry-item",
            "source_item_id": "failed-item",
            "canonical_task_ids": ["TASK-FIXTURE-001"],
            "retry_type": "verification_evidence_repair",
            "sourceResidualGapIds": ["gap-one"],
            "failurePredicate": "expected value equals observed value",
            "focused_verification": ["compare values"],
            "expected_evidence": ["gap-one resolved"]
        }),
        None,
    );

    assert!(normalized.issues.is_empty(), "{:?}", normalized.issues);
    assert_eq!(normalized.value["source_residual_gap_ids"][0], "gap-one");
    assert_eq!(
        normalized.value["failed_predicate"],
        "expected value equals observed value"
    );
}
