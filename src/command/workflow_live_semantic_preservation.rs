//! D74 — host-enforced semantic preservation for LLM repair adoption.
//!
//! Repair reducers fix SHAPE; they must not rewrite the semantic identity of
//! the work being repaired. Every repair-boundary prompt already states this
//! contract ("preserve ... verbatim; do not drop, invent, or reclassify");
//! this module is the host-side enforcement. A repair that mutates or drops a
//! protected field on a matched item — or drops an identified item outright —
//! is rejected and the pre-repair value stays authoritative. Re-authoring a
//! predicate is legal only through the explicit
//! `predicate_unsatisfiable_as_written` route, and even then the source gap
//! identity must survive.

use std::collections::BTreeSet;

use serde_json::Value;

/// Fields that identify WHAT failed and WHY. A repair may reorganize items or
/// fill in missing schedulable fields; it may never rewrite these on an item
/// the host already knows.
const PROTECTED_FIELDS: &[&str] = &[
    "source_residual_gap_ids",
    "failed_predicate",
    "classification",
    "canonical_task_ids",
    "source_item_id",
    "failure_status",
    "failure_evidence",
];

/// Fields a repair may re-author when it explicitly declares the
/// `predicate_unsatisfiable_as_written` route. Gap identity
/// (`source_residual_gap_ids`), task identity, and source binding are never
/// re-authorable.
const REAUTHORABLE_WITH_ROUTE: &[&str] =
    &["failed_predicate", "classification", "failure_evidence"];

const MAX_REPORTED_VIOLATIONS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreservationCheck {
    pub(crate) violations: Vec<String>,
}

impl PreservationCheck {
    pub(crate) fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Compare the pre-repair items against the repaired items. Every original
/// item that carries an identity must survive with its protected fields
/// intact; items without any identity cannot be tracked and are skipped.
pub(crate) fn check_items(original: &[Value], repaired: &[Value]) -> PreservationCheck {
    let mut violations = Vec::new();
    for item in original {
        let Some(label) = item_label(item) else {
            continue;
        };
        let Some(matched) = matching_repaired_item(item, repaired) else {
            violations.push(format!(
                "repair dropped item '{label}' without the explicit predicate_unsatisfiable_as_written route"
            ));
            continue;
        };
        check_protected_fields(&label, item, matched, &mut violations);
        if violations.len() >= MAX_REPORTED_VIOLATIONS {
            break;
        }
    }
    violations.truncate(MAX_REPORTED_VIOLATIONS);
    PreservationCheck { violations }
}

/// The canonical top-level route arrays of a harvested triage/retriage value.
pub(crate) fn canonical_route_entries(triage: &Value) -> Vec<Value> {
    let data = triage
        .get("data")
        .or_else(|| triage.get("result").and_then(|result| result.get("data")))
        .unwrap_or(triage);
    let mut entries = Vec::new();
    for key in [
        "retry_items",
        "implementation_failures",
        "superseded_items",
        "terminal_blockers",
    ] {
        if let Some(values) = data.get(key).and_then(Value::as_array) {
            entries.extend(values.iter().cloned());
        }
    }
    entries
}

/// Feed rejection reasons back into an inventory's `unresolved_issues` so the
/// next bounded repair attempt sees exactly what it violated.
pub(crate) fn append_preservation_issues(inventory: &mut Value, violations: &[String]) {
    let Some(object) = inventory.as_object_mut() else {
        return;
    };
    let issues = object
        .entry("unresolved_issues".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(issues) = issues.as_array_mut() else {
        return;
    };
    issues.extend(violation_issues(violations));
}

/// Violations as typed issue records for repair-attempt evidence.
pub(crate) fn violation_issues(violations: &[String]) -> Vec<Value> {
    violations
        .iter()
        .map(|violation| {
            serde_json::json!({
                "kind": "semantic_preservation",
                "field": "items",
                "message": violation,
            })
        })
        .collect()
}

fn check_protected_fields(
    label: &str,
    original: &Value,
    repaired: &Value,
    violations: &mut Vec<String>,
) {
    let reauthored = reauthoring_declared(repaired);
    for field in PROTECTED_FIELDS {
        let Some(original_value) = original.get(*field).filter(|value| value_present(value)) else {
            continue;
        };
        if reauthored && REAUTHORABLE_WITH_ROUTE.contains(field) {
            continue;
        }
        let matches = repaired
            .get(*field)
            .is_some_and(|value| field_equal(original_value, value));
        if !matches {
            violations.push(format!(
                "repair mutated or dropped protected field '{field}' on item '{label}'"
            ));
        }
    }
}

fn reauthoring_declared(item: &Value) -> bool {
    ["route", "classification", "verification_failure_class"]
        .iter()
        .filter_map(|key| item.get(*key).and_then(Value::as_str))
        .any(|value| {
            value
                .to_ascii_lowercase()
                .contains("predicate_unsatisfiable")
        })
}

fn matching_repaired_item<'a>(original: &Value, repaired: &'a [Value]) -> Option<&'a Value> {
    let keys = identity_keys(original);
    repaired.iter().find(|candidate| {
        let candidate_keys = identity_keys(candidate);
        keys.iter().any(|key| candidate_keys.contains(key))
    })
}

fn identity_keys(item: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for field in ["item_id", "id"] {
        if let Some(id) = trimmed_str(item.get(field)) {
            keys.insert(format!("item:{id}"));
        }
    }
    if let Some(id) = trimmed_str(item.get("source_item_id")) {
        keys.insert(format!("source:{id}"));
    }
    let tasks = string_set(item.get("canonical_task_ids"));
    if !tasks.is_empty() {
        keys.insert(format!(
            "tasks:{}",
            tasks.into_iter().collect::<Vec<_>>().join(",")
        ));
    }
    keys
}

fn item_label(item: &Value) -> Option<String> {
    if identity_keys(item).is_empty() {
        return None;
    }
    trimmed_str(item.get("item_id"))
        .or_else(|| trimmed_str(item.get("id")))
        .or_else(|| trimmed_str(item.get("source_item_id")))
        .map(str::to_string)
        .or_else(|| {
            let tasks = string_set(item.get("canonical_task_ids"));
            (!tasks.is_empty()).then(|| tasks.into_iter().collect::<Vec<_>>().join(","))
        })
}

fn field_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::String(left), Value::String(right)) => left.trim() == right.trim(),
        (Value::Array(_), Value::Array(_)) => {
            let left_set = string_set(Some(left));
            let right_set = string_set(Some(right));
            if left_set.is_empty() && right_set.is_empty() {
                return left == right;
            }
            left_set == right_set
        }
        _ => left == right,
    }
}

fn value_present(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Null => false,
        _ => true,
    }
}

fn string_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect()
}

fn trimmed_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn retry_item() -> Value {
        json!({
            "item_id": "retry-1",
            "source_item_id": "verify-TASK-A-001",
            "canonical_task_ids": ["TASK-A-001"],
            "source_residual_gap_ids": ["gap-1"],
            "failed_predicate": "report.status == passed",
            "classification": "retryable_verification_shape_issue",
        })
    }

    #[test]
    fn identical_items_pass() {
        let original = vec![retry_item()];
        let repaired = vec![retry_item()];
        assert!(check_items(&original, &repaired).passed());
    }

    #[test]
    fn mutated_predicate_is_rejected() {
        let original = vec![retry_item()];
        let mut item = retry_item();
        item["failed_predicate"] = json!("report exists");
        let check = check_items(&original, &[item]);
        assert!(!check.passed());
        assert!(check.violations[0].contains("failed_predicate"));
    }

    #[test]
    fn dropped_gap_ids_are_rejected() {
        let original = vec![retry_item()];
        let mut item = retry_item();
        item.as_object_mut()
            .unwrap()
            .remove("source_residual_gap_ids");
        let check = check_items(&original, &[item]);
        assert!(!check.passed());
        assert!(check.violations[0].contains("source_residual_gap_ids"));
    }

    #[test]
    fn dropped_item_is_rejected() {
        let original = vec![retry_item()];
        let check = check_items(&original, &[]);
        assert!(!check.passed());
        assert!(check.violations[0].contains("dropped item"));
    }

    #[test]
    fn declared_reauthoring_route_allows_predicate_rewrite_but_keeps_gap_ids() {
        let original = vec![retry_item()];
        let mut reauthored = retry_item();
        reauthored["route"] = json!("predicate_unsatisfiable_as_written");
        reauthored["failed_predicate"] = json!("runtime evidence contradicts predicate");
        reauthored["classification"] = json!("predicate_unsatisfiable_as_written");
        assert!(check_items(&original, &[reauthored.clone()]).passed());

        reauthored
            .as_object_mut()
            .unwrap()
            .remove("source_residual_gap_ids");
        let check = check_items(&original, &[reauthored]);
        assert!(!check.passed());
        assert!(check.violations[0].contains("source_residual_gap_ids"));
    }

    #[test]
    fn task_reassignment_is_rejected() {
        let original = vec![retry_item()];
        let mut item = retry_item();
        item["canonical_task_ids"] = json!(["TASK-A-002"]);
        // Still matched by item_id, but the task identity changed.
        let check = check_items(&original, &[item]);
        assert!(!check.passed());
        assert!(check.violations[0].contains("canonical_task_ids"));
    }

    #[test]
    fn unidentified_original_items_are_skipped() {
        let original = vec![json!({"summary": "free text row"})];
        assert!(check_items(&original, &[]).passed());
    }

    #[test]
    fn consolidation_by_task_signature_matches() {
        let mut original = retry_item();
        original.as_object_mut().unwrap().remove("item_id");
        original.as_object_mut().unwrap().remove("source_item_id");
        let mut repaired = retry_item();
        repaired["item_id"] = json!("consolidated-1");
        repaired.as_object_mut().unwrap().remove("source_item_id");
        assert!(check_items(&[original], &[repaired]).passed());
    }

    #[test]
    fn route_entries_read_data_and_root_shapes() {
        let triage = json!({
            "data": {
                "retry_items": [{"item_id": "r1"}],
                "implementation_failures": [{"item_id": "f1"}],
            }
        });
        assert_eq!(canonical_route_entries(&triage).len(), 2);
        let root = json!({"terminal_blockers": [{"id": "t1"}]});
        assert_eq!(canonical_route_entries(&root).len(), 1);
    }

    #[test]
    fn preservation_issues_are_appended_to_inventories() {
        let mut inventory = json!({"items": [], "unresolved_issues": []});
        append_preservation_issues(&mut inventory, &["violation one".to_string()]);
        let issues = inventory["unresolved_issues"].as_array().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["kind"], "semantic_preservation");
    }
}
