use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn enforce_retry_invariants(inventory: &Value, verification: &Value) -> Value {
    let failed = support::non_accepted_outcomes(&support::outcomes_of(verification));
    if failed.is_empty() {
        return inventory.clone();
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    let mut issues = support::array(object.get("unresolved_issues"));
    issues.retain(|issue| !retry_invariant_contract_issue(issue));
    let mut items = Vec::new();
    for mut item in support::array(object.get("items")) {
        if let Some(source) = matching_failed_outcome(&item, &failed) {
            stamp_retry_invariant(&mut item, source);
        } else if self_contained_retry_matches_failed_task(&item, &failed) {
            // Re-triage and repair-plan reducers may identify the implementation
            // source while merged verification outcomes carry generated check IDs.
            // Preserve a complete retry invariant when the failed task identity
            // still overlaps instead of orphaning otherwise schedulable work.
        } else {
            issues.push(invariant_issue(
                &item,
                "source_item_id",
                "retry item does not identify a failed source outcome",
            ));
        }
        items.push(item);
    }
    object.insert("items".to_string(), Value::Array(items));
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

fn self_contained_retry_matches_failed_task(item: &Value, failed: &[Value]) -> bool {
    let gaps = support::strings_of(item.get("source_residual_gap_ids"));
    let predicate = item
        .get("failed_predicate")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if gaps.is_empty() || !predicate {
        return false;
    }
    let item_tasks = retry_task_ids(item);
    !item_tasks.is_empty()
        && failed.iter().any(|outcome| {
            retry_task_ids(outcome)
                .iter()
                .any(|id| item_tasks.contains(id))
        })
}

fn retry_task_ids(value: &Value) -> Vec<String> {
    let mut ids = support::strings_of(value.get("canonical_task_ids"));
    ids.extend(support::strings_of(
        value
            .get("result")
            .and_then(|result| result.get("data"))
            .and_then(|data| data.get("canonical_task_ids")),
    ));
    support::unique(ids)
}

fn retry_invariant_contract_issue(issue: &Value) -> bool {
    matches!(
        issue.get("field").and_then(Value::as_str),
        Some("source_residual_gap_ids" | "failed_predicate")
    )
}

fn stamp_retry_invariant(item: &mut Value, source: &Value) {
    let gaps = residual_gap_entries(source);
    let Some(object) = item.as_object_mut() else {
        return;
    };
    object.insert(
        "source_residual_gap_ids".to_string(),
        serde_json::json!(gaps.iter().map(|(id, _)| id).collect::<Vec<_>>()),
    );
    object.insert(
        "failed_predicate".to_string(),
        Value::String(source_failed_predicate(source, &gaps)),
    );
}

fn source_failed_predicate(source: &Value, gaps: &[(String, String)]) -> String {
    let descriptions = gaps
        .iter()
        .map(|(_, description)| description.trim())
        .filter(|description| !description.is_empty())
        .collect::<Vec<_>>();
    if !descriptions.is_empty() {
        return descriptions.join("\n");
    }
    source
        .get("result")
        .and_then(|result| result.get("summary"))
        .or_else(|| source.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn matching_failed_outcome<'a>(item: &Value, failed: &'a [Value]) -> Option<&'a Value> {
    let item_ids = verification_item_ids(item);
    failed.iter().find(|outcome| {
        verification_item_ids(outcome)
            .iter()
            .any(|id| item_ids.contains(id))
    })
}

pub(super) fn residual_gap_entries(value: &Value) -> Vec<(String, String)> {
    residual_gap_roots(value)
        .into_iter()
        .flat_map(|root| support::array(root.get("residual_gaps")))
        .filter_map(|gap| {
            Some((
                gap.get("id")?.as_str()?.to_string(),
                gap.get("description")?.as_str()?.trim().to_string(),
            ))
        })
        .collect()
}

fn residual_gap_roots(value: &Value) -> Vec<&Value> {
    let mut roots = vec![value];
    if let Some(result) = value.get("result") {
        roots.push(result);
    }
    roots
}

pub(super) fn verification_item_ids(value: &Value) -> Vec<String> {
    let mut ids = direct_item_ids(value);
    if let Some(data) = value.get("result").and_then(|result| result.get("data")) {
        ids.extend(direct_item_ids(data));
    }
    support::unique(ids)
}

fn direct_item_ids(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["item_id", "id", "source_item_id", "split_from_item_id"] {
        if let Some(id) = value.get(key).and_then(Value::as_str) {
            ids.push(id.to_string());
        }
    }
    ids.extend(support::strings_of(value.get("source_outcome_item_ids")));
    ids
}

fn invariant_issue(item: &Value, field: &str, message: &str) -> Value {
    serde_json::json!({
        "kind": "evidence_repair",
        "field": field,
        "message": message,
        "item_id": item.get("item_id").or_else(|| item.get("id")).cloned(),
        "canonical_task_ids": support::strings_of(item.get("canonical_task_ids")),
    })
}
