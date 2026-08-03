use super::*;

pub(super) fn normalize_retry_invariant_context(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let gap_ids = raw_strings_from_aliases(
        value,
        &[
            "source_residual_gap_ids",
            "sourceResidualGapIds",
            "residual_gap_ids",
            "residualGapIds",
        ],
    );
    if !gap_ids.is_empty() {
        object.insert(
            "source_residual_gap_ids".to_string(),
            serde_json::json!(gap_ids),
        );
    }
    let predicate = first_string(
        value,
        &[
            "failed_predicate",
            "failedPredicate",
            "failure_predicate",
            "failurePredicate",
        ],
    );
    if let Some(predicate) = predicate {
        object.insert(
            "failed_predicate".to_string(),
            serde_json::json!(predicate.clone()),
        );
        append_alias_values(
            object,
            "expected_evidence",
            vec![serde_json::json!(predicate)],
        );
    }
}

pub(super) fn retry_invariant_missing_fields(value: &serde_json::Value) -> Vec<&'static str> {
    if !generated_retry_verification_item(value) {
        return Vec::new();
    }
    let mut missing = Vec::new();
    if !value_present(value.get("source_residual_gap_ids")) {
        missing.push("source_residual_gap_ids");
    }
    if !value_present(value.get("failed_predicate")) {
        missing.push("failed_predicate");
    }
    missing
}

fn generated_retry_verification_item(value: &serde_json::Value) -> bool {
    let retry_metadata = [
        "retry_type",
        "retryType",
        "retry_reason",
        "retryReason",
        "repair_type",
        "repairType",
        "source_failed_item_id",
        "sourceFailedItemId",
        "source_outcome_item_ids",
        "sourceOutcomeItemIds",
        "retry_steps",
        "retrySteps",
        "recommended_retry",
        "recommendedRetry",
    ]
    .iter()
    .any(|key| value_present(value.get(*key)));
    let retry_class = value
        .get("classification")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|class| class.to_ascii_lowercase().contains("retry"));
    generated_focused_verification_item(value) && (retry_metadata || retry_class)
}
