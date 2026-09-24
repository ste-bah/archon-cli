//! The writable scope a TASK declares, admitted to the item that implements
//! it.
//!
//! # The deadlock this ends
//!
//! Two host rules read two different answers to "what may this branch write".
//!
//! - The verifier is told the task's scope from the task universe: every path
//!   the task file declares as its own, stamped by
//!   [`crate::v2::verification::path_ownership`], including the entries the
//!   author wrote as absolute paths. A defect inside that scope is the task's
//!   to fix, and the verifier is instructed not to excuse it.
//! - The write branch is given the item's `target_files` — whatever the
//!   workflow script declared for the call — widened only by module expansion
//!   and by the deliverable contracts
//!   ([`crate::v2::contract_code_targets`]). The task's own declaration is
//!   never consulted.
//!
//! When the script's list is narrower than the task's, the two answers
//! disagree permanently and the task cannot terminate: the verifier refuses
//! acceptance naming a file inside the declared scope, the write gate drops
//! every edit to that same file as undeclared, and each remediation attempt
//! reports "accepted" having changed nothing that matters. Observed live: a
//! task whose file declared six paths was dispatched with one; six
//! verification attempts named the same two declared files, six remediation
//! branches were refused them at the gate, and the attempt budget ran out
//! with the task unable to pass or fail.
//!
//! The absorbing property is what makes it fatal rather than merely slow. A
//! declared file that is never granted can never be changed, so it never
//! enters the set derived from what the branch changed, so it is never
//! granted — no number of attempts moves it.
//!
//! # The floor, not the ceiling
//!
//! The task declaration is a FLOOR: it is unioned with what the item already
//! carries, never substituted for it. A script may legitimately declare more
//! than the task file lists — a module the work splits into, a path an
//! earlier attempt created — and those stay. What it may not do is declare
//! less than the task owns, because the verifier judges the task by the task
//! file.
//!
//! # What is NOT admitted
//!
//! - Anything outside this item's OWN tasks. Only the tasks the item claims
//!   contribute, so a path another task declares is refused exactly as it is
//!   today; the gate's whole purpose is to stop a branch trampling another
//!   task's files and that survives untouched.
//! - A path under a project artifact root. Those are produced artifacts,
//!   admitted by the artifact path instead, and repository targets are for
//!   repository code.
//! - An entry that names no single repository path: prose with no path in it,
//!   a template token, a glob, a parent traversal, or an absolute path
//!   outside the repository root. Nothing is invented from an entry this host
//!   cannot read.
//! - Anything at all for an item that owns no repository code. Such an item
//!   is artifact-only and is served by the artifact path; handing it code
//!   writes would change what the branch is for.
//!
//! Host-parsed declarations, the run's artifact roots and the repository root
//! only. No task, PRD, language or domain knowledge.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::contract_code_targets::admissible_repository_path;
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// Repository paths this item's tasks declare and the item does not already
/// list, repository-relative and in declaration order.
pub(crate) fn task_declared_code_targets_for_item(
    universe: &WorkflowV2TaskUniverse,
    item: &Value,
    artifact_roots: &[String],
    repository_root: Option<&Path>,
) -> Vec<String> {
    let declared = declared_repository_targets(item, repository_root);
    task_declared_repository_paths(universe, item, artifact_roots, repository_root)
        .into_iter()
        .filter(|path| !declared.contains(path))
        .collect()
}

/// Every repository path this item's tasks declare, whether or not the item
/// already lists it.
///
/// The whole floor, for a caller that is REPLACING an item's targets rather
/// than adding to them: once the floor has been stamped onto the item, the
/// difference against the item is empty, and a caller that asked for the
/// difference would rebuild the scope without the floor it just gained.
pub(crate) fn task_declared_repository_paths(
    universe: &WorkflowV2TaskUniverse,
    item: &Value,
    artifact_roots: &[String],
    repository_root: Option<&Path>,
) -> Vec<String> {
    // An item with no repository targets is artifact-only; see the module doc.
    if declared_repository_targets(item, repository_root).is_empty() {
        return Vec::new();
    }
    let task_ids = canonical_task_ids(item);
    if task_ids.is_empty() {
        return Vec::new();
    }
    let mut paths: Vec<String> = Vec::new();
    for task in &universe.tasks {
        if !task_ids.iter().any(|id| id == &task.canonical_task_id) {
            continue;
        }
        for entry in &task.files_expected_to_change {
            let Some(path) = repository_relative_declaration(entry, repository_root) else {
                continue;
            };
            let Some(path) = admissible_repository_path(&path, artifact_roots) else {
                continue;
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// One declared entry as a repository-relative path, or `None` when it names
/// no single repository path this host can resolve.
///
/// The entries are prose as often as paths — a path followed by a dash and a
/// note about it — so the path is taken with the same reader the planner and
/// the ownership stamp use, and the result is then read by the same spelling
/// rule: authors write these absolute as freely as relative, and the grant is
/// relative, so an absolute entry under the repository root becomes its
/// relative form rather than being dropped as unreadable.
fn repository_relative_declaration(entry: &str, repository_root: Option<&Path>) -> Option<String> {
    let parsed = crate::v2::script::declared_path(entry)?;
    if names_no_single_file(&parsed) {
        return None;
    }
    let Some(root) = repository_root else {
        // With no root an absolute entry cannot be resolved, and joining it
        // under a guessed root would invent a target. Relative entries are
        // already in the grant's language.
        return (!Path::new(parsed.trim()).is_absolute()).then(|| parsed.trim().to_string());
    };
    let DeclaredPathForm::Repo(path) = declared_path_form(&parsed, root) else {
        return None;
    };
    // A target is matched by containment, so a directory admitted as one
    // would grant every file beneath it — the "grant everything" answer,
    // arrived at one bullet at a time. A path that is a directory on disk is
    // not a file this task writes; one that does not exist yet is a file it
    // has still to create.
    (!root.join(&path).is_dir()).then_some(path)
}

/// Whether the entry cannot name one repository file: a trailing separator, a
/// glob, or embedded whitespace — the same shapes the write-call pre-flight
/// refuses in an authored target list, applied to an authored declaration.
fn names_no_single_file(parsed: &str) -> bool {
    let trimmed = parsed.trim();
    trimmed.is_empty()
        || trimmed.ends_with('/')
        || trimmed.ends_with('\\')
        || trimmed.chars().any(char::is_whitespace)
        || trimmed.contains(['*', '?', '['])
}

/// The item's existing targets in the grant's own language, so an entry
/// already declared under either spelling is not added a second time.
fn declared_repository_targets(item: &Value, repository_root: Option<&Path>) -> BTreeSet<String> {
    item.get("target_files")
        .and_then(Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|target| match repository_root {
                    Some(root) => match declared_path_form(target, root) {
                        DeclaredPathForm::Repo(path) => Some(path),
                        DeclaredPathForm::Outside | DeclaredPathForm::Unusable => None,
                    },
                    None => {
                        let trimmed = target.trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    }
                })
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
#[path = "task_declared_targets_tests.rs"]
mod tests;
