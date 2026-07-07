//! Outcome/evidence matching helpers for the Rust decomposed-PRD lifecycle —
//! the noop.js family plus body_a.js acceptance/readiness helpers. Child of
//! the lifecycle support module (re-exported from there).

use std::collections::BTreeSet;

use serde_json::Value;

use super::{LifecycleContract, array, present, strings_of};

/// JS `hasConcreteEvidence` (body_a.js).
pub(crate) fn has_concrete_evidence(outcome: &Value) -> bool {
    match outcome.get("evidence") {
        None | Some(Value::Null) => {
            present(outcome.get("completion_evidence"))
                || present(outcome.get("artifact_paths"))
                || present(outcome.get("artifacts"))
                || present(outcome.get("task_coverage"))
                || present(outcome.get("commands_run"))
        }
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(other) => present(Some(other)),
    }
}

pub(crate) fn outcome_status(outcome: &Value) -> Option<&str> {
    outcome.get("status").and_then(Value::as_str)
}

pub(crate) fn outcome_accepted_or_noop(outcome: &Value) -> bool {
    matches!(outcome_status(outcome), Some("accepted") | Some("noop"))
}

/// JS `acceptedOrNoopCanonicalTaskIdsFrom`.
pub(crate) fn accepted_or_noop_canonical_task_ids_from(
    contract: &LifecycleContract<'_>,
    outcomes: &[Value],
) -> Vec<String> {
    let mut ids = Vec::new();
    for outcome in outcomes {
        if !outcome_accepted_or_noop(outcome) || !has_concrete_evidence(outcome) {
            continue;
        }
        ids.extend(contract.canonical_ids_for(outcome));
    }
    ids
}

/// JS `nonAcceptedOutcomes`.
pub(crate) fn non_accepted_outcomes(outcomes: &[Value]) -> Vec<Value> {
    outcomes
        .iter()
        .filter(|outcome| !outcome_accepted_or_noop(outcome))
        .cloned()
        .collect()
}

/// JS `matchingAcceptedIds`.
pub(crate) fn matching_accepted_ids(
    contract: &LifecycleContract<'_>,
    source_items: &[Value],
    outcomes: &[Value],
) -> Vec<String> {
    let mut allowed = BTreeSet::new();
    for item in source_items {
        for id in contract.canonical_ids_for(item) {
            allowed.insert(id);
        }
    }
    accepted_or_noop_canonical_task_ids_from(contract, outcomes)
        .into_iter()
        .filter(|id| allowed.contains(id))
        .collect()
}

/// JS `outcomes` accessor: `result.outcomes || result.items || [result]`.
pub(crate) fn outcomes_of(result: &Value) -> Vec<Value> {
    let outcomes = array(result.get("outcomes"));
    if !outcomes.is_empty() {
        return outcomes;
    }
    let items = array(result.get("items"));
    if !items.is_empty() {
        return items;
    }
    vec![result.clone()]
}

pub(crate) fn work_type_for(item: &Value) -> &str {
    item.get("work_type").and_then(Value::as_str).unwrap_or("")
}

/// JS `validImplementationItem` / `validVerifiedNoopItem` / `validInventoryItem`.
pub(crate) fn valid_inventory_item(contract: &LifecycleContract<'_>, item: &Value) -> bool {
    let has_id = item.get("item_id").is_some() || item.get("id").is_some();
    let canonical_ok = !contract.canonical_ids_for(item).is_empty()
        && contract.invalid_dependency_ids_for(item).is_empty();
    let artifact_declared = item.get("artifact_requirements").is_some();
    match work_type_for(item) {
        "implementation" => {
            has_id
                && canonical_ok
                && present(item.get("target_files"))
                && present(item.get("acceptance_criteria"))
                && present(item.get("focused_verification"))
                && artifact_declared
        }
        "verified_noop" => {
            has_id
                && canonical_ok
                && present(item.get("acceptance_criteria"))
                && present(item.get("noop_proof"))
                && present(item.get("noop_proof_refs"))
                && artifact_declared
        }
        _ => false,
    }
}

/// JS `readyItemsFrom`.
pub(crate) fn ready_items_from(
    contract: &LifecycleContract<'_>,
    items: &[Value],
    completed: &BTreeSet<String>,
) -> Vec<Value> {
    items
        .iter()
        .filter(|item| {
            contract
                .dependency_ids_for(item)
                .iter()
                .all(|id| completed.contains(id))
        })
        .cloned()
        .collect()
}

/// JS `itemIsCompleted`.
pub(crate) fn item_is_completed(
    contract: &LifecycleContract<'_>,
    item: &Value,
    completed: &BTreeSet<String>,
) -> bool {
    let ids = contract.canonical_ids_for(item);
    !ids.is_empty() && ids.iter().all(|id| completed.contains(id))
}

/// JS noop.js: `outcomeHasNoopSourceEvidence`.
fn outcome_has_noop_source_evidence(source_item: &Value, outcome: &Value) -> bool {
    if !outcome_accepted_or_noop(outcome) || !has_concrete_evidence(outcome) {
        return false;
    }
    if array(source_item.get("artifact_requirements")).is_empty() {
        return true;
    }
    [
        "artifacts",
        "artifact_paths",
        "artifacts_checked",
        "current_artifacts_checked",
        "commands_run",
        "current_commands_run",
        "completion_evidence",
    ]
    .iter()
    .any(|key| !array(outcome.get(*key)).is_empty())
}

/// JS noop.js: `matchingAcceptedNoopIds`.
pub(crate) fn matching_accepted_noop_ids(
    contract: &LifecycleContract<'_>,
    source_items: &[Value],
    outcomes: &[Value],
) -> Vec<String> {
    let mut accepted = BTreeSet::new();
    for item in source_items {
        let source_ids: BTreeSet<String> = contract.canonical_ids_for(item).into_iter().collect();
        for outcome in outcomes {
            if !outcome_has_noop_source_evidence(item, outcome) {
                continue;
            }
            for id in contract.canonical_ids_for(outcome) {
                if source_ids.contains(&id) {
                    accepted.insert(id);
                }
            }
        }
    }
    accepted.into_iter().collect()
}

/// JS noop.js: `matchingAcceptedCompletionIds`.
pub(crate) fn matching_accepted_completion_ids(
    contract: &LifecycleContract<'_>,
    source_items: &[Value],
    outcomes: &[Value],
) -> Vec<String> {
    let mut accepted = Vec::new();
    for item in source_items {
        let ids = if work_type_for(item) == "verified_noop" {
            matching_accepted_noop_ids(contract, std::slice::from_ref(item), outcomes)
        } else {
            matching_accepted_ids(contract, std::slice::from_ref(item), outcomes)
        };
        for id in ids {
            if !accepted.contains(&id) {
                accepted.push(id);
            }
        }
    }
    accepted
}
