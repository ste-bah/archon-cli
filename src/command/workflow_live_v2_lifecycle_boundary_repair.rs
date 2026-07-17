use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

const OUTCOME_ITEM_ALIASES: &[&str] = &[
    "items",
    "remediation_items",
    "remediationItems",
    "retry_items",
    "retryItems",
];
const OUTCOME_CONTAINER_KEYS: &[&str] = &["items", "remediation", "inventory", "routes"];
const RECONCILIATION_ITEM_ALIASES: &[&str] = &[
    "items",
    "issues",
    "evidence_issues",
    "evidenceIssues",
    "reconciliation_items",
    "reconciliationItems",
];
const RECONCILIATION_CONTAINER_KEYS: &[&str] = &["items", "reconciliation", "evidence", "issues"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct OutcomeRepairQuality {
    pub(super) unaccounted: usize,
    pub(super) unresolved_issues: usize,
    pub(super) empty_inventory: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReconciliationQuality {
    pub(super) missing_collection: usize,
    pub(super) malformed_items: usize,
}

impl ReconciliationQuality {
    pub(super) fn defect_count(self) -> usize {
        self.missing_collection + self.malformed_items
    }
}

pub(super) fn harvest_outcome_repair_items(value: &Value) -> Value {
    harvest_known_collection(value, "items", OUTCOME_ITEM_ALIASES, OUTCOME_CONTAINER_KEYS)
}

pub(super) fn harvest_reconciliation_items(value: &Value) -> Value {
    harvest_known_collection(
        value,
        "items",
        RECONCILIATION_ITEM_ALIASES,
        RECONCILIATION_CONTAINER_KEYS,
    )
}

pub(super) fn collection_items(value: &Value) -> Vec<Value> {
    support::array(collection_data(value).get("items"))
}

pub(super) fn outcome_repair_quality(
    inventory: &Value,
    failed_outcomes: &[Value],
) -> OutcomeRepairQuality {
    let items = collection_items(inventory);
    let accounted_ids = items
        .iter()
        .flat_map(super::workflow_live_v2_lifecycle_verify_invariants::verification_item_ids)
        .collect::<std::collections::BTreeSet<_>>();
    let unaccounted = failed_outcomes
        .iter()
        .filter(|outcome| {
            !super::workflow_live_v2_lifecycle_verify_invariants::verification_item_ids(outcome)
                .iter()
                .any(|id| accounted_ids.contains(id))
        })
        .count();
    OutcomeRepairQuality {
        unaccounted,
        unresolved_issues: support::array(collection_data(inventory).get("unresolved_issues"))
            .len(),
        empty_inventory: usize::from(items.is_empty()),
    }
}

pub(super) fn reconciliation_quality(value: &Value) -> ReconciliationQuality {
    let data = collection_data(value);
    let Some(items) = data.get("items").and_then(Value::as_array) else {
        return ReconciliationQuality {
            missing_collection: 1,
            malformed_items: 0,
        };
    };
    ReconciliationQuality {
        missing_collection: 0,
        malformed_items: items.iter().filter(|item| !item.is_object()).count(),
    }
}

fn harvest_known_collection(
    value: &Value,
    canonical_key: &str,
    aliases: &[&str],
    container_keys: &[&str],
) -> Value {
    let mut harvested = value.clone();
    let Some(data) = collection_data_mut(&mut harvested) else {
        return harvested;
    };
    let mut items = aliases
        .iter()
        .flat_map(|key| array_values(data.get(*key)))
        .collect::<Vec<_>>();
    for container_key in container_keys {
        let Some(container) = data.get(*container_key).and_then(Value::as_object) else {
            continue;
        };
        for alias in aliases {
            items.extend(array_values(container.get(*alias)));
        }
    }
    dedup_items(&mut items);
    if !items.is_empty() || data.get(canonical_key).is_some_and(Value::is_array) {
        data.insert(canonical_key.to_string(), Value::Array(items));
    }
    harvested
}

fn array_values(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn collection_data(value: &Value) -> &Value {
    value
        .get("data")
        .or_else(|| value.get("result").and_then(|result| result.get("data")))
        .unwrap_or(value)
}

fn collection_data_mut(value: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    let has_data = value.get("data").is_some_and(Value::is_object);
    let has_result_data = value
        .get("result")
        .and_then(|result| result.get("data"))
        .is_some_and(Value::is_object);
    if has_data {
        return value.get_mut("data").and_then(Value::as_object_mut);
    }
    if has_result_data {
        return value
            .get_mut("result")
            .and_then(|result| result.get_mut("data"))
            .and_then(Value::as_object_mut);
    }
    value.as_object_mut()
}

fn dedup_items(items: &mut Vec<Value>) {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| seen.insert(serde_json::to_string(item).unwrap_or_default()));
}

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_boundary_repair_tests.rs"]
mod tests;
