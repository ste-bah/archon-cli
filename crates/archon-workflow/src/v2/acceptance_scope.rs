//! Batch E2: the product area a task set covers, and so the only place an
//! acceptance remediation may be handed a file no task declares.
//!
//! Batch E granted a failing check's implicated file to its remediation unit
//! whenever no task declared it. Live on wf-0ddadd81 the target repository
//! also holds the harness that runs the workflow, and rustc warnings in a
//! check's output named harness sources: four of them were granted to three
//! remediation units. "No task declares it" is not "safe to hand a worker":
//! an unowned file is grantable only inside the plan's scope roots.
//!
//! The roots are read from the task universe's declared entries alone
//! (`files_expected_to_change`, shared-append targets, deliverable contract
//! paths), each read as a repository-relative path; an entry that is not one
//! widens nothing. For each declared path:
//!
//! - the path itself (a declared directory covers everything under it);
//! - a declared file's module directory, its path less the extension
//!   (`src/x.rs` -> `src/x/`), where a split of that file lands;
//! - the nearest package root holding it, STRICTLY below the repository
//!   root (`write::scope_roots::package_root`): a declared file in
//!   `crates/lib-a/src/` puts `crates/lib-a/` in the product area.
//!
//! Unlike the write stage's ceiling there is no top-level-directory
//! fallback: an unpackaged `src/command/x.rs` makes `src/command/x.rs` and
//! `src/command/x/` the roots, never `src/`, which in a repository holding
//! its own harness is the harness. And unlike that ceiling, a plan that
//! declares nothing has no roots and covers nothing.

use std::collections::BTreeSet;
use std::path::Path;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_covers, declared_path_form, declared_paths_of,
};

/// The union of every task's scope roots, repository-relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanScopeRoots(BTreeSet<String>);

impl PlanScopeRoots {
    /// The roots of `universe`'s declared entries under `root`.
    pub fn of(universe: &WorkflowV2TaskUniverse, root: &Path) -> Self {
        let mut roots = BTreeSet::new();
        for task in &universe.tasks {
            for declared in declared_paths_of(task) {
                let DeclaredPathForm::Repo(path) = declared_path_form(&declared, root) else {
                    continue;
                };
                let path = path.trim_end_matches("/**").trim_end_matches('/');
                if path.is_empty() || path.contains('*') {
                    continue;
                }
                roots.extend(roots_of(path, root));
            }
        }
        Self(roots)
    }

    /// Whether `file` lies inside one of the roots.
    pub fn covers(&self, file: &str) -> bool {
        self.0.iter().any(|root| declared_covers(root, file))
    }

    /// The roots, sorted.
    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// The roots one declared repository path contributes.
fn roots_of(path: &str, root: &Path) -> Vec<String> {
    let mut roots = vec![path.to_string()];
    let (parent, name) = match path.rsplit_once('/') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, path),
    };
    // A declared file's module directory: `x.rs` splits into `x/`.
    if let Some((stem, _)) = name.rsplit_once('.')
        && !stem.is_empty()
    {
        roots.push(match parent {
            Some(parent) => format!("{parent}/{stem}"),
            None => stem.to_string(),
        });
    }
    let start = if root.join(path).is_dir() {
        Some(path)
    } else {
        parent
    };
    if let Some(package) =
        start.and_then(|dir| crate::v2::write::scope_roots::package_root(root, dir))
    {
        roots.push(package);
    }
    roots
}

#[cfg(test)]
#[path = "acceptance_scope_tests.rs"]
mod tests;
