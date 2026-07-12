use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) struct VerificationTriageRoutes {
    pub(super) implementation_failures: Vec<Value>,
    pub(super) retry_items: Vec<Value>,
}

pub(super) fn triage_routes(triage: &Value) -> VerificationTriageRoutes {
    let data = triage_data(triage);
    let mut implementation_failures = support::array(data.get("implementation_failures"));
    implementation_failures.extend(
        support::array(data.get("items"))
            .into_iter()
            .filter(is_actionable_classification),
    );
    dedup_items(&mut implementation_failures);
    VerificationTriageRoutes {
        implementation_failures,
        retry_items: support::array(data.get("retry_items")),
    }
}

pub(super) fn write_remediation_outcomes(repair_plan: &Value, verification: &Value) -> Vec<Value> {
    let data = repair_plan
        .get("data")
        .or_else(|| {
            repair_plan
                .get("result")
                .and_then(|result| result.get("data"))
        })
        .unwrap_or(repair_plan);
    if data.get("route").and_then(Value::as_str) != Some("write_remediation") {
        return Vec::new();
    }
    let requested = route_outcome_ids(data);
    support::non_accepted_outcomes(&support::outcomes_of(verification))
        .into_iter()
        .filter(|outcome| requested.is_empty() || requested.contains(&outcome_id(outcome)))
        .collect()
}

pub(super) fn predicate_rewrite_inventory(
    repair_plan: &Value,
    verification: &Value,
) -> Option<Value> {
    let data = triage_data(repair_plan);
    if data.get("route").and_then(Value::as_str) != Some("predicate_unsatisfiable_as_written") {
        return None;
    }
    let mut items = support::array(data.get("re_authored_items"));
    if items.is_empty() {
        items = support::array(data.get("items"));
    }
    let outcomes = support::non_accepted_outcomes(&support::outcomes_of(verification));
    for item in &mut items {
        stamp_failed_predicate(item, &outcomes);
    }
    Some(serde_json::json!({ "status": "accepted", "items": items }))
}

fn stamp_failed_predicate(item: &mut Value, outcomes: &[Value]) {
    let source_id = item
        .get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(outcome) = outcomes.iter().find(|outcome| {
        let id = outcome_id(outcome);
        !source_id.is_empty() && (id == source_id || id.ends_with(&format!("-{source_id}")))
    }) else {
        return;
    };
    let gaps = residual_gaps(outcome);
    let ids: Vec<String> = gaps
        .iter()
        .filter_map(|gap| gap.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let predicate = gaps
        .first()
        .and_then(|gap| gap.get("description"))
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = item.as_object_mut() {
        object.insert(
            "source_residual_gap_ids".to_string(),
            serde_json::json!(ids),
        );
        object.insert("failed_predicate".to_string(), predicate);
    }
}

fn residual_gaps(outcome: &Value) -> Vec<Value> {
    let result = outcome.get("result").unwrap_or(outcome);
    support::array(result.get("residual_gaps"))
}

fn route_outcome_ids(data: &Value) -> Vec<String> {
    [
        "source_outcome_ids",
        "affected_source_outcome_ids",
        "outcome_ids",
    ]
    .iter()
    .flat_map(|key| support::strings_of(data.get(*key)))
    .collect()
}

fn outcome_id(outcome: &Value) -> String {
    outcome
        .get("item_id")
        .or_else(|| outcome.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn triage_data(triage: &Value) -> &Value {
    triage
        .get("data")
        .or_else(|| triage.get("result").and_then(|result| result.get("data")))
        .unwrap_or(triage)
}

fn is_actionable_classification(item: &Value) -> bool {
    let class = item
        .get("classification")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    class.contains("actionable") || class.contains("implementation_failure")
}

fn dedup_items(items: &mut Vec<Value>) {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| seen.insert(item.to_string()));
}

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_routing_tests.rs"]
mod tests;
