//! Seed inventory items' `artifact_requirements` from the task universe's own
//! parsed deliverable contracts.
//!
//! The validator requires `artifact_requirements` to hold concrete artifact
//! paths. Tasks that declare their deliverables as `deliverable_contracts:` —
//! rather than under an `artifact_requirements:` key or an "Artifact
//! Requirements" heading — produce items with the field empty, and the repair
//! loop then asks a reducer to reconstruct those paths from prose.
//!
//! It cannot reliably do that, and it does not have to: the host already parsed
//! them. Observed live: seven items flagged, the payload handed to the reducer
//! carried 96 `artifact_path` values across 15 `deliverable_contracts` blocks,
//! and six consecutive repairs resolved none of the seven before the cap
//! blocked the run. An earlier run ended the same way on a different agent.
//!
//! The paths are copied here instead, deterministically, before validation runs.
//!
//! A contract path that the item ALSO declares as a repository target is code,
//! not a project artifact — a code task's deliverables are its source files —
//! so those are left out. That test uses only the item's own declarations, so
//! it carries no task, PRD, language or domain knowledge.

use serde_json::Value;

use crate::task_universe::WorkflowV2TaskUniverse;

/// Fill in each item's `artifact_requirements` from its tasks' declared
/// deliverable contracts, for items that declare none.
pub(crate) fn seed_artifact_requirements(
    universe: &WorkflowV2TaskUniverse,
    inventory: &Value,
) -> Value {
    let mut seeded = inventory.clone();
    let Some(items) = seeded.get_mut("items").and_then(Value::as_array_mut) else {
        return seeded;
    };
    for item in items {
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        if declares_requirements(object.get("artifact_requirements")) {
            continue;
        }
        let targets = declared_targets(object.get("target_files"));
        let paths: Vec<Value> = contract_paths_for(universe, object.get("canonical_task_ids"))
            .into_iter()
            .filter(|path| !targets.contains(path))
            .map(Value::String)
            .collect();
        if paths.is_empty() {
            continue;
        }
        object.insert("artifact_requirements".to_string(), Value::Array(paths));
    }
    seeded
}

/// Concrete, non-templated contract paths for the item's canonical tasks, in
/// declaration order and de-duplicated.
fn contract_paths_for(universe: &WorkflowV2TaskUniverse, task_ids: Option<&Value>) -> Vec<String> {
    let wanted: Vec<&str> = task_ids
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut paths: Vec<String> = Vec::new();
    for task in &universe.tasks {
        if !wanted.iter().any(|id| *id == task.canonical_task_id) {
            continue;
        }
        for contract in &task.deliverable_contracts {
            let path = contract.artifact_path.trim();
            // A template is not a concrete path; stamping one would assert a
            // literal `${VAR}` deliverable.
            if path.is_empty() || path.contains("${") || paths.iter().any(|seen| seen == path) {
                continue;
            }
            paths.push(path.to_string());
        }
    }
    paths
}

fn declared_targets(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .map(|target| target.trim().to_string())
                .filter(|target| !target.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// An absent, null or empty list declares nothing — the state this seeding
/// exists to fill.
fn declares_requirements(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(requirements)) => !requirements.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "inventory_artifact_seeding_tests.rs"]
mod tests;
