//! Batch O: the host's own requirement inventory for a run's coverage audit.
//!
//! The runtime coverage audit was asked for "requirements no individual task
//! claims" while seeing only findings and a roster, so it could not answer.
//! This is the host's answer, computed from the frozen task set and the PRD,
//! never from agent text: every PRD obligation id with its exact text, the
//! tasks whose `implements` claim it, the frozen checks that name it (by id,
//! or in a check's `covers` list when the contract carries one), and the
//! host-computed gaps -- obligations no task claims, obligations no check
//! names, claims of ids the PRD does not define, and tasks that claim
//! nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};

use crate::obligation_ids::{acceptance_criteria, obligation_texts};
use crate::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, AcceptanceContract};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The inventory as one JSON document (shape in the module doc):
/// `{requirements: [{id, text, claimed_by, checked_by}], claim_map,
/// unclaimed, unchecked, phantom_claims: [{task_id, id}],
/// tasks_without_claims, check_ids}`. Sorted throughout, so the same inputs
/// give the same bytes.
pub fn requirement_inventory(
    prd_text: &str,
    universe: &WorkflowV2TaskUniverse,
    contract: Option<&AcceptanceContract>,
) -> Value {
    let mut texts = obligation_texts(prd_text);
    for (id, criterion) in acceptance_criteria(prd_text) {
        texts.entry(id).or_insert(criterion);
    }
    let mut claim_map: BTreeMap<String, BTreeSet<String>> = texts
        .keys()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect();
    let mut phantom = Vec::new();
    let mut idle = Vec::new();
    for task in &universe.tasks {
        if task.implements.is_empty() {
            idle.push(task.canonical_task_id.clone());
        }
        for id in &task.implements {
            match claim_map.get_mut(id.trim()) {
                Some(claimants) => {
                    claimants.insert(task.canonical_task_id.clone());
                }
                None => phantom.push(json!({"task_id": task.canonical_task_id, "id": id})),
            }
        }
    }
    let checks = contract.map(checks_by_requirement).unwrap_or_default();
    let check_ids: BTreeSet<&String> = checks.values().flatten().collect();
    let requirements: Vec<Value> = texts
        .iter()
        .map(|(id, text)| {
            json!({
                "id": id,
                "text": text,
                "claimed_by": claim_map.get(id).cloned().unwrap_or_default(),
                "checked_by": checks.get(id).cloned().unwrap_or_default(),
            })
        })
        .collect();
    let unclaimed: Vec<&String> = claim_map
        .iter()
        .filter(|(_, tasks)| tasks.is_empty())
        .map(|(id, _)| id)
        .collect();
    let unchecked: Vec<&String> = texts
        .keys()
        .filter(|id| !checks.contains_key(*id))
        .collect();
    json!({
        "requirements": requirements,
        "claim_map": claim_map,
        "unclaimed": unclaimed,
        "unchecked": unchecked,
        "phantom_claims": phantom,
        "tasks_without_claims": idle,
        "check_ids": check_ids,
    })
}

/// [`requirement_inventory`] read from disk: the PRD at `prd_path` and the
/// acceptance contract frozen in `tasks_root`, when there is one. `Err`
/// when the PRD cannot be read or a contract present does not parse.
pub fn requirement_inventory_from_files(
    prd_path: &Path,
    tasks_root: &Path,
    universe: &WorkflowV2TaskUniverse,
) -> Result<Value, String> {
    let prd = std::fs::read_to_string(prd_path)
        .map_err(|err| format!("{} unreadable: {err}", prd_path.display()))?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract = match std::fs::read(&contract_path) {
        Ok(bytes) => Some(
            serde_json::from_slice::<AcceptanceContract>(&bytes)
                .map_err(|err| format!("{} is not a contract: {err}", contract_path.display()))?,
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(format!("{} unreadable: {err}", contract_path.display())),
    };
    Ok(requirement_inventory(&prd, universe, contract.as_ref()))
}

/// Requirement id -> the frozen checks naming it: by the check's own id, or
/// in its `covers` list (read from the serialized entry, so a contract that
/// carries the field is honoured and one that does not reads as ids only).
fn checks_by_requirement(contract: &AcceptanceContract) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        map.entry(entry.id.clone())
            .or_default()
            .insert(entry.id.clone());
        let covers = serde_json::to_value(entry)
            .ok()
            .and_then(|value| value.get("covers").cloned());
        for id in covers
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            map.entry(id.to_string())
                .or_default()
                .insert(entry.id.clone());
        }
    }
    map
}

#[cfg(test)]
#[path = "requirement_inventory_tests.rs"]
mod tests;
