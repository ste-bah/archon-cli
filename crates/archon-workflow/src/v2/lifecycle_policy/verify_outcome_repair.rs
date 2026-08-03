use serde_json::Value;
use std::collections::BTreeSet;

use crate::generated_lifecycle_support as support;

use super::verify_merge;

pub fn repairable_contract_outcomes(remediation_wave: &Value) -> Vec<Value> {
    support::non_accepted_outcomes(&support::outcomes_of(remediation_wave))
        .into_iter()
        .filter(is_contract_failure)
        .collect()
}

pub fn merge_repaired_outcomes(
    remediation_wave: &Value,
    followup_wave: Value,
    followup_items: &[Value],
) -> Value {
    verify_merge::merge_repair_outcomes(remediation_wave, followup_wave, followup_items)
}

pub fn next_noop_disagreement_streak(
    previous: usize,
    before: &Value,
    after: &Value,
    followup: &Value,
) -> usize {
    let signatures = contract_failure_signatures(before);
    if !signatures.is_empty()
        && signatures == contract_failure_signatures(after)
        && followup_is_all_noop(followup)
    {
        previous + 1
    } else {
        0
    }
}

pub fn mark_noop_disagreement(remediation_wave: &Value) -> Value {
    let signatures = contract_failure_signatures(remediation_wave);
    let outcomes = support::outcomes_of(remediation_wave)
        .into_iter()
        .map(|outcome| mark_if_unchanged(outcome, &signatures))
        .collect();
    verify_merge::replace_all_outcomes(
        remediation_wave,
        outcomes,
        "verification remediation overreach",
    )
}

fn contract_failure_signatures(wave: &Value) -> BTreeSet<String> {
    repairable_contract_outcomes(wave)
        .iter()
        .map(contract_failure_signature)
        .collect()
}

fn contract_failure_signature(outcome: &Value) -> String {
    let id = outcome
        .get("item_id")
        .or_else(|| outcome.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error = outcome
        .get("error")
        .or_else(|| outcome.pointer("/result/summary"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{id}\n{error}")
}

fn followup_is_all_noop(followup: &Value) -> bool {
    let outcomes = support::outcomes_of(followup);
    !outcomes.is_empty()
        && outcomes
            .iter()
            .all(|outcome| outcome.get("status").and_then(Value::as_str) == Some("noop"))
}

fn mark_if_unchanged(mut outcome: Value, signatures: &BTreeSet<String>) -> Value {
    if !signatures.contains(&contract_failure_signature(&outcome)) {
        return outcome;
    }
    let Some(object) = outcome.as_object_mut() else {
        return outcome;
    };
    object.insert(
        "failure_kind".to_string(),
        Value::String("verification_overreach".to_string()),
    );
    object.insert(
        "error".to_string(),
        Value::String(
            "unchanged verification failure survived two noop remediation rounds".to_string(),
        ),
    );
    if let Some(result) = object.get_mut("result").and_then(Value::as_object_mut)
        && let Some(data) = result.get_mut("data").and_then(Value::as_object_mut)
    {
        data.insert(
            "verification_failure_class".to_string(),
            Value::String("artifact_contract_overreach".to_string()),
        );
        data.insert(
            "verification_remediation_required".to_string(),
            Value::Bool(false),
        );
    }
    outcome
}

fn is_contract_failure(outcome: &Value) -> bool {
    let failure_kind = outcome
        .get("failure_kind")
        .or_else(|| outcome.pointer("/result/data/failure_kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(
        failure_kind.trim().to_ascii_lowercase().as_str(),
        "contract" | "format" | "complexity" | "code_hygiene"
    )
}
