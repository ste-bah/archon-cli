use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

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

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_routing_tests.rs"]
mod tests;
