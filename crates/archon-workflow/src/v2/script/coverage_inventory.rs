//! Batch O: the coverage audit's requirement inventory, from the HOST.
//!
//! The coverage audit's reduce was asked for "requirements no individual
//! task claims" while it saw only findings and a roster. Now the prelude asks
//! the host, on a checkpoint carrying [`REQUIREMENT_INVENTORY_MARKER`], for
//! the PRD's requirements and the frozen task set's claim map
//! ([`crate::requirement_inventory`]); the view carries them under
//! [`REQUIREMENT_INVENTORY_KEY`], computed at the moment of asking.
//!
//! And the host adds to the coverage audit's final set, itself, one finding
//! per requirement no task claims and per requirement claimed only by tasks
//! the audit never reviewed (a blocked task): no reviewer has to notice them.
//!
//! The task set is the universe's source root; the PRD is the one its
//! frozen acceptance contract names, resolved against the nearest ancestor
//! of the task set that holds it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Result};
use crate::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, AcceptanceContract};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The checkpoint option that asks for the inventory.
pub const REQUIREMENT_INVENTORY_MARKER: &str = "requirementInventory";
/// Key of the inventory in that checkpoint's view.
pub const REQUIREMENT_INVENTORY_KEY: &str = "requirement_inventory";
/// The `source` of every finding the host adds from the inventory.
pub const INVENTORY_FINDING_SOURCE: &str = "host-requirement-inventory";

/// The inventory for `universe`'s task set, or why there is none.
pub fn inventory(universe: Option<&WorkflowV2TaskUniverse>) -> Result<Value, String> {
    let universe = universe.ok_or("there is no task universe")?;
    let tasks_root = universe
        .source_roots
        .first()
        .map(PathBuf::from)
        .ok_or("the task universe names no task set")?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(&contract_path)
            .map_err(|err| format!("{} unreadable: {err}", contract_path.display()))?,
    )
    .map_err(|err| format!("{} is not a contract: {err}", contract_path.display()))?;
    let prd = prd_path(&tasks_root, &contract.prd.path).ok_or_else(|| {
        format!(
            "the PRD `{}` the contract names was not found",
            contract.prd.path
        )
    })?;
    crate::requirement_inventory::requirement_inventory_from_files(&prd, &tasks_root, universe)
}

fn prd_path(tasks_root: &Path, named: &str) -> Option<PathBuf> {
    let path = PathBuf::from(named);
    if path.is_absolute() {
        return path.is_file().then_some(path);
    }
    tasks_root
        .ancestors()
        .map(|dir| dir.join(&path))
        .find(|candidate| candidate.is_file())
}

/// The inventory as the script reads it: each requirement's text, the
/// claim map, each task's claims, and the unclaimed ids.
fn script_view(universe: Option<&WorkflowV2TaskUniverse>) -> Value {
    let inventory = match inventory(universe) {
        Ok(inventory) => inventory,
        Err(why) => return json!({"source": "host", "unavailable": why}),
    };
    let mut by_task: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (id, tasks) in inventory["claim_map"].as_object().into_iter().flatten() {
        for task in tasks
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            by_task
                .entry(task.to_string())
                .or_default()
                .push(id.clone());
        }
    }
    let requirements: Vec<Value> = inventory["requirements"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|req| json!({"id": req["id"], "text": req["text"]}))
        .collect();
    json!({"source": "host", "requirements": requirements, "claims": inventory["claim_map"],
        "by_task": by_task, "unclaimed": inventory["unclaimed"],
        "phantom_claims": inventory["phantom_claims"]})
}

/// `result` with the inventory, for the view of the checkpoint that asked;
/// `None` for every other record. The key is the host's alone.
pub fn with_requirement_inventory(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> Option<WorkflowV2Result> {
    let asks = record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(REQUIREMENT_INVENTORY_MARKER) == Some(&Value::Bool(true));
    if !asks && result.data.get(REQUIREMENT_INVENTORY_KEY).is_none() {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(REQUIREMENT_INVENTORY_KEY);
    }
    if asks {
        viewed.data[REQUIREMENT_INVENTORY_KEY] = script_view(universe);
    }
    Some(viewed)
}

/// The findings the host adds to the coverage audit's final set: one per
/// requirement no task claims (naming no task: the remediation plan routes
/// it by content) and one per requirement whose every claimant is outside
/// `reviewed` (naming those claimants). None when there is no inventory.
pub fn inventory_findings(
    universe: Option<&WorkflowV2TaskUniverse>,
    reviewed: &BTreeSet<String>,
) -> Vec<Value> {
    let Ok(inventory) = inventory(universe) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for req in inventory["requirements"].as_array().into_iter().flatten() {
        let id = req["id"].as_str().unwrap_or_default();
        let text = req["text"].as_str().unwrap_or_default();
        let claimants: Vec<&str> = req["claimed_by"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if claimants.is_empty() {
            findings.push(json!({"id": format!("unclaimed-{id}"), "requirement_id": id,
                "severity": "high", "source": INVENTORY_FINDING_SOURCE,
                "claim": format!("PRD requirement {id} is claimed by no task, so nothing implements or checks it: {text}")}));
        } else if claimants.iter().all(|task| !reviewed.contains(*task)) {
            findings.push(json!({"id": format!("unreviewed-claim-{id}"), "requirement_id": id,
                "canonical_task_ids": claimants, "severity": "high", "source": INVENTORY_FINDING_SOURCE,
                "claim": format!("PRD requirement {id} is claimed only by task(s) the coverage audit never reviewed ({}), so nothing shows it is met: {text}", claimants.join(", "))}));
        }
    }
    findings
}

#[cfg(test)]
#[path = "coverage_inventory_tests.rs"]
mod tests;
