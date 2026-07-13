use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::support;

pub(super) fn verification_remediation_source_items(inventory: &Value) -> Vec<Value> {
    support::array(inventory.get("items"))
        .into_iter()
        .map(stamp_verification_requirements)
        .collect()
}

pub(super) fn merge_retry_outcomes(
    verification: &Value,
    retry_result: Value,
    retry_items: &[Value],
) -> Value {
    merge_selected_outcomes(
        verification,
        retry_result,
        retry_items,
        "verification retry",
    )
}

pub(super) fn merge_repair_outcomes(
    remediation_wave: &Value,
    followup_wave: Value,
    followup_items: &[Value],
) -> Value {
    merge_selected_outcomes(
        remediation_wave,
        followup_wave,
        followup_items,
        "remediation repair",
    )
}

fn merge_selected_outcomes(
    original: &Value,
    replacement: Value,
    replacement_items: &[Value],
    label: &str,
) -> Value {
    let replaced_ids = retry_source_ids(replacement_items);
    let mut outcomes: Vec<Value> = support::outcomes_of(original)
        .into_iter()
        .filter(|outcome| !outcome_matches(outcome, &replaced_ids))
        .collect();
    outcomes.extend(support::outcomes_of(&replacement));
    verification_with_outcomes(original, outcomes, label)
}

fn stamp_verification_requirements(item: Value) -> Value {
    let Some(mut object) = item.as_object().cloned() else {
        return item;
    };
    if support::present(object.get("verification_requirements")) {
        return Value::Object(object);
    }
    let requirements = support::array(object.get("focused_verification"));
    object.insert(
        "verification_requirements".to_string(),
        Value::Array(requirements),
    );
    Value::Object(object)
}

fn retry_source_ids(items: &[Value]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for item in items {
        let before = ids.len();
        add_id(item.get("source_item_id"), &mut ids);
        add_ids(item.get("source_outcome_item_ids"), &mut ids);
        if ids.len() == before {
            add_id(item.get("item_id"), &mut ids);
        }
    }
    ids
}

fn add_id(value: Option<&Value>, ids: &mut BTreeSet<String>) {
    if let Some(id) = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        ids.insert(id.to_string());
    }
}

fn add_ids(value: Option<&Value>, ids: &mut BTreeSet<String>) {
    for id in support::strings_of(value) {
        ids.insert(id);
    }
}

fn outcome_matches(outcome: &Value, ids: &BTreeSet<String>) -> bool {
    ["item_id", "id", "source_item_id"]
        .into_iter()
        .filter_map(|key| outcome.get(key).and_then(Value::as_str))
        .any(|candidate| ids.iter().any(|id| id_matches(candidate, id)))
}

fn id_matches(candidate: &str, expected: &str) -> bool {
    candidate == expected || candidate.ends_with(&format!("-{expected}"))
}

fn verification_with_outcomes(verification: &Value, outcomes: Vec<Value>, label: &str) -> Value {
    let mut merged = verification.as_object().cloned().unwrap_or_default();
    let status = merged_status(&outcomes);
    let summary = merged_summary(&outcomes, label);
    merged.insert("outcomes".to_string(), Value::Array(outcomes.clone()));
    merged.insert("items".to_string(), Value::Array(outcomes.clone()));
    merged.insert("status".to_string(), Value::String(status.to_string()));
    merged.insert("summary".to_string(), Value::String(summary.clone()));
    update_nested_result(&mut merged, &outcomes, status, &summary);
    Value::Object(merged)
}

fn merged_status(outcomes: &[Value]) -> &'static str {
    if outcomes.iter().all(support::outcome_accepted_or_noop) {
        "accepted"
    } else {
        "needs_review"
    }
}

fn merged_summary(outcomes: &[Value], label: &str) -> String {
    let unresolved = outcomes
        .iter()
        .filter(|outcome| !support::outcome_accepted_or_noop(outcome))
        .count();
    format!(
        "{label} merged {} outcomes with {unresolved} unresolved",
        outcomes.len()
    )
}

fn update_nested_result(
    merged: &mut Map<String, Value>,
    outcomes: &[Value],
    status: &str,
    summary: &str,
) {
    let Some(result) = merged.get_mut("result").and_then(Value::as_object_mut) else {
        return;
    };
    result.insert("status".to_string(), Value::String(status.to_string()));
    result.insert("summary".to_string(), Value::String(summary.to_string()));
    let data = result
        .entry("data")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(data) = data.as_object_mut() else {
        return;
    };
    data.insert("outcomes".to_string(), Value::Array(outcomes.to_vec()));
    data.insert("items".to_string(), Value::Array(outcomes.to_vec()));
}
