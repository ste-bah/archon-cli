//! Repository deliverables a CODE item declares, admitted to its writable
//! targets.
//!
//! A task states what it produces in `deliverable_contracts`. When one of those
//! is a repository source file, the item that owns the task must be able to
//! write it — but `target_files` and `deliverable_contracts` are separate
//! fields, and only the first governed the patch. So the agent was told to
//! produce the file, produced it, and the write layer dropped it: it was not a
//! declared target, so it never entered the patch and died with the worktree.
//!
//! Observed live. One task's declared targets were six files;
//! `data_store/coverage.rs` and `data_store/coverage_tests.rs` were declared as
//! contracts instead. The agent edited both, the branch reported
//! `patch_landed: true` for the other six, and the completion check then failed
//! it for the two the write layer had just discarded. No number of attempts
//! could pass: producing them was in scope, keeping them was not.
//!
//! [`contract_artifact_paths_for_item`] already admits declared contracts for
//! ARTIFACT-ONLY items. This is the other half — the same host-parsed
//! declaration, for items that also own repository code.
//!
//! Reads the task universe and the run's artifact roots, both host-owned. A
//! contract under an artifact root is a project artifact and stays out of the
//! repository targets; everything else is repository work the task declared for
//! itself. No task, PRD, language or domain knowledge.
//!
//! [`contract_artifact_paths_for_item`]: super::project_artifact_contract_roots

use serde_json::Value;

use crate::task_universe::WorkflowV2TaskUniverse;

/// Repository deliverable paths this item declares and does not already list.
pub(crate) fn contract_code_targets_for_item(
    universe: &WorkflowV2TaskUniverse,
    item: &Value,
    artifact_roots: &[String],
) -> Vec<String> {
    let declared = declared_targets(item);
    // An item with no repository targets is artifact-only; that case is
    // already served, and admitting code paths here would hand it writes it
    // was never scoped for.
    if declared.is_empty() {
        return Vec::new();
    }
    let task_ids = canonical_task_ids(item);
    if task_ids.is_empty() {
        return Vec::new();
    }
    let mut added: Vec<String> = Vec::new();
    for task in &universe.tasks {
        if !task_ids.iter().any(|id| id == &task.canonical_task_id) {
            continue;
        }
        for contract in &task.deliverable_contracts {
            let Some(path) = admissible_repository_path(&contract.artifact_path, artifact_roots)
            else {
                continue;
            };
            if !declared.contains(&path) && !added.contains(&path) {
                added.push(path);
            }
        }
    }
    added
}

/// A concrete repository-relative path, or `None` when the declaration names a
/// project artifact or cannot name one file.
fn admissible_repository_path(raw: &str, artifact_roots: &[String]) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("${")
        || trimmed.contains('*')
        || trimmed.contains('<')
        || trimmed.ends_with('/')
        // `has_root()` as well as `is_absolute()`. On Windows `/etc/passwd`
        // carries no drive letter, so `is_absolute()` is FALSE and an absolute
        // path went down the relative branch to be joined under the repository
        // root instead of refused — an escape guard failing open on one
        // platform. `archon-write-plan::normalize_target` already carries this
        // exact fix; these newer readers repeated the narrow check.
        || std::path::Path::new(trimmed).is_absolute()
        || std::path::Path::new(trimmed).has_root()
    {
        return None;
    }
    let segments: Vec<&str> = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() || segments.contains(&"..") {
        return None;
    }
    let path = segments.join("/");
    if under_artifact_root(&path, artifact_roots) {
        return None;
    }
    Some(path)
}

/// Artifact roots are where produced artifacts live. A contract under one is a
/// project artifact, not repository code, and is served by the artifact path.
fn under_artifact_root(path: &str, artifact_roots: &[String]) -> bool {
    artifact_roots.iter().any(|root| {
        let root = root.trim().trim_end_matches('/');
        !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
    })
}

fn declared_targets(item: &Value) -> Vec<String> {
    item.get("target_files")
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

fn canonical_task_ids(item: &Value) -> Vec<String> {
    item.get("canonical_task_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "contract_code_targets_tests.rs"]
mod tests;
