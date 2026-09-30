//! Batch O: the run's scope amendments (`crate::task_scope_amendment`) are
//! in force for every write branch.
//!
//! Two effects, both read from the run's verified amendment ledger, never
//! from agent text:
//!
//! - the task universe the fan-out plans with is the AMENDED one, so the
//!   declared-scope floor (`stamp_task_declared_targets`), the owner claims
//!   and the forbidden lists all treat a granted file as its grantee's
//!   declared file;
//! - each project-data grant of a branch's tasks becomes a declared project
//!   artifact of that branch (`project_artifact_requirements`), so its copy
//!   is seeded, captured and landed through the audited project-input
//!   landing -- baseline, stale refusal, kept copy, append-only log -- and
//!   never through the repository patch, where a path git ignores would be
//!   skipped.
//!
//! A ledger that exists but does not verify stops the call: silently
//! planning without its grants would drop work the host already granted.

use crate::error::{WorkflowError, WorkflowResult};
use crate::task_scope_amendment::{ScopeAmendmentLedger, amended_universe, project_data_grants};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::WorkflowV2ResultStore;

/// Stamp every branch's project-data grants, and return the amended
/// universe when the run amended anything (`None`: plan with the frozen
/// one).
pub(super) fn apply(
    branches: &mut [crate::WorkflowV2FanoutItem],
    v2_store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> WorkflowResult<Option<WorkflowV2TaskUniverse>> {
    let Some(run_root) = v2_store.root().parent() else {
        return Ok(None);
    };
    let ledger = ScopeAmendmentLedger::load(run_root)
        .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
    if ledger.set.grants.is_empty() {
        return Ok(None);
    }
    for branch in branches.iter_mut() {
        stamp_project_data(branch, &ledger);
    }
    Ok(universe.map(|universe| amended_universe(universe, &ledger.set)))
}

fn stamp_project_data(branch: &mut crate::WorkflowV2FanoutItem, ledger: &ScopeAmendmentLedger) {
    let Some(item) = branch
        .input
        .get_mut("item")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let task_ids: Vec<String> = item
        .get("canonical_task_ids")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    let grants = project_data_grants(&ledger.set, &task_ids);
    if grants.is_empty() {
        return;
    }
    let entry = item
        .entry("project_artifact_requirements")
        .or_insert_with(|| serde_json::json!([]));
    if !entry.is_array() {
        // A single declared path: kept, beside the grants.
        let single = entry.take();
        *entry = match single {
            serde_json::Value::Null => serde_json::json!([]),
            other => serde_json::json!([other]),
        };
    }
    let list = entry.as_array_mut().expect("an array by construction");
    for path in grants {
        let value = serde_json::Value::String(path);
        if !list.contains(&value) {
            list.push(value);
        }
    }
}

#[cfg(test)]
#[path = "scope_amendment_stamps_tests.rs"]
mod tests;
