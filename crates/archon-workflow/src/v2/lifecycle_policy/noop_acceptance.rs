use std::collections::BTreeSet;

use serde_json::Value;

use crate::generated_lifecycle_support::{self as support, LifecycleContract};

pub fn pin_noop_acceptance_criteria(
    contract: &LifecycleContract<'_>,
    items: &[Value],
) -> Vec<Value> {
    items
        .iter()
        .map(|item| {
            let criteria = authoritative_criteria_for_item(contract, item);
            let mut object = item.as_object().cloned().unwrap_or_default();
            object.insert(
                "acceptance_criteria".to_string(),
                serde_json::json!(
                    criteria
                        .iter()
                        .map(|(_, criterion)| criterion)
                        .collect::<Vec<_>>()
                ),
            );
            object.insert(
                "task_acceptance_criteria".to_string(),
                serde_json::json!(
                    criteria
                        .iter()
                        .map(|(task_id, criterion)| {
                            serde_json::json!({
                                "task_id": task_id,
                                "criterion": criterion,
                            })
                        })
                        .collect::<Vec<_>>()
                ),
            );
            Value::Object(object)
        })
        .collect()
}

pub fn enforce_noop_acceptance_criteria(
    contract: &LifecycleContract<'_>,
    source_items: &[Value],
    outcomes: &[Value],
) -> Vec<Value> {
    outcomes
        .iter()
        .cloned()
        .map(|outcome| {
            if !support::outcome_accepted_or_noop(&outcome) {
                return outcome;
            }
            let Some(source_item) = source_items
                .iter()
                .find(|item| items_share_task(contract, item, &outcome))
            else {
                return refute_noop_criteria(
                    outcome,
                    &["no authoritative source item matched the accepted no-op outcome".into()],
                );
            };
            let expected = authoritative_criteria_for_item(contract, source_item);
            let results = acceptance_criteria_results(&outcome);
            let missing = expected
                .iter()
                .filter(|(task_id, criterion)| {
                    !results.iter().any(|result| {
                        result_matches_criterion(result, task_id, criterion)
                            && criterion_result_passed(result)
                    })
                })
                .map(|(task_id, criterion)| format!("{task_id}: {criterion}"))
                .collect::<Vec<_>>();
            if expected.is_empty() {
                return refute_noop_criteria(
                    outcome,
                    &["authoritative TASK file declared no parseable acceptance criteria".into()],
                );
            }
            if missing.is_empty() {
                outcome
            } else {
                refute_noop_criteria(outcome, &missing)
            }
        })
        .collect()
}

fn authoritative_criteria_for_item(
    contract: &LifecycleContract<'_>,
    item: &Value,
) -> Vec<(String, String)> {
    let task_ids = contract.canonical_ids_for(item);
    contract
        .task_universe
        .tasks
        .iter()
        .filter(|task| task_ids.contains(&task.canonical_task_id))
        .flat_map(|task| {
            task.acceptance_criteria
                .iter()
                .cloned()
                .map(|criterion| (task.canonical_task_id.clone(), criterion))
        })
        .collect()
}

fn items_share_task(contract: &LifecycleContract<'_>, left: &Value, right: &Value) -> bool {
    let left = contract
        .canonical_ids_for(left)
        .into_iter()
        .collect::<BTreeSet<_>>();
    contract
        .canonical_ids_for(right)
        .iter()
        .any(|id| left.contains(id))
}

fn acceptance_criteria_results(outcome: &Value) -> Vec<Value> {
    let mut results = Vec::new();
    for root in [
        Some(outcome),
        outcome.get("data"),
        outcome.get("result"),
        outcome.get("result").and_then(|result| result.get("data")),
    ]
    .into_iter()
    .flatten()
    {
        results.extend(support::array(root.get("acceptance_criteria_results")));
        results.extend(support::array(root.get("criterion_results")));
    }
    results
}

fn result_matches_criterion(result: &Value, task_id: &str, criterion: &str) -> bool {
    let result_criterion = result
        .get("criterion")
        .or_else(|| result.get("acceptance_criterion"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let result_task_id = result
        .get("task_id")
        .or_else(|| result.get("canonical_task_id"))
        .and_then(Value::as_str);
    result_criterion == criterion.trim()
        && result_task_id.is_none_or(|result_task_id| result_task_id == task_id)
}

fn criterion_result_passed(result: &Value) -> bool {
    result
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "accepted" | "complete" | "completed" | "pass" | "passed" | "satisfied"
            )
        })
        && support::present(result.get("evidence_refs"))
}

fn refute_noop_criteria(mut outcome: Value, missing: &[String]) -> Value {
    let gap = serde_json::json!({
        "id": "noop-acceptance-criteria-unsatisfied",
        "description": format!(
            "accepted/noop proof did not explicitly satisfy every authoritative TASK acceptance criterion: {}",
            missing.join(" | ")
        ),
        "severity": "blocking",
        "missing_criteria": missing,
    });
    let Some(object) = outcome.as_object_mut() else {
        return outcome;
    };
    object.insert("status".into(), Value::String("needs_review".into()));
    let mut gaps = support::array(object.get("residual_gaps"));
    gaps.push(gap.clone());
    object.insert("residual_gaps".into(), Value::Array(gaps));
    if let Some(result) = object.get_mut("result").and_then(Value::as_object_mut) {
        result.insert("status".into(), Value::String("needs_review".into()));
        let mut result_gaps = support::array(result.get("residual_gaps"));
        result_gaps.push(gap);
        result.insert("residual_gaps".into(), Value::Array(result_gaps));
    }
    outcome
}
