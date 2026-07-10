use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn enforce_retry_invariants(inventory: &Value, verification: &Value) -> Value {
    let failed = support::non_accepted_outcomes(&support::outcomes_of(verification));
    if failed.is_empty() {
        return inventory.clone();
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    let mut issues = support::array(object.get("unresolved_issues"));
    for item in support::array(object.get("items")) {
        issues.extend(retry_item_invariant_issues(&item, &failed));
    }
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

fn retry_item_invariant_issues(item: &Value, failed: &[Value]) -> Vec<Value> {
    let Some(source) = matching_failed_outcome(item, failed) else {
        return vec![invariant_issue(
            item,
            "source_item_id",
            "retry item does not identify a failed source outcome",
        )];
    };
    let gaps = residual_gap_entries(source);
    if gaps.is_empty() {
        return vec![invariant_issue(
            source,
            "residual_gaps",
            "failed source outcome has no invariant identity",
        )];
    }
    let source_ids = support::strings_of(item.get("source_residual_gap_ids"));
    let missing: Vec<String> = gaps
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| !source_ids.contains(id))
        .collect();
    let mut issues = Vec::new();
    if !missing.is_empty() {
        issues.push(invariant_issue_for_gaps(
            item,
            "source_residual_gap_ids",
            &format!(
                "retry item dropped source residual gap IDs: {}",
                missing.join(", ")
            ),
            &gaps,
        ));
    }
    if !predicate_matches_gaps(item, &gaps) {
        issues.push(invariant_issue_for_gaps(
            item,
            "failed_predicate",
            "retry item changed or dropped the failed predicate",
            &gaps,
        ));
    }
    issues
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

pub(super) fn predicate_matches_gaps(item: &Value, gaps: &[(String, String)]) -> bool {
    let Some(predicate) = item.get("failed_predicate").and_then(Value::as_str) else {
        return false;
    };
    gaps.iter()
        .filter(|(_, description)| !description.is_empty())
        .all(|(_, description)| predicate.contains(description))
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

fn invariant_issue_for_gaps(
    item: &Value,
    field: &str,
    message: &str,
    gaps: &[(String, String)],
) -> Value {
    let mut issue = invariant_issue(item, field, message);
    issue["required_source_residual_gap_ids"] =
        serde_json::json!(gaps.iter().map(|(id, _)| id).collect::<Vec<_>>());
    issue["required_failed_predicate"] = serde_json::json!(gap_predicate(gaps));
    issue
}

fn gap_predicate(gaps: &[(String, String)]) -> String {
    gaps.iter()
        .map(|(_, description)| description.trim())
        .filter(|description| !description.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
