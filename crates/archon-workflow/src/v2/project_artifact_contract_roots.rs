//! Artifact roots derived from what the tasks themselves declare.
//!
//! The allowed project-artifact roots were hardcoded (`.archon/artifacts` plus
//! run directories), and agent-supplied requirement paths are deliberately
//! restricted to `.archon/`-prefixed roots. Neither channel could ever admit a
//! deliverable like `docs/<area>/report.md` — so a task whose *contract*
//! declared exactly that had no writable home: the write was classified as a
//! repository change outside declared targets and refused as unsafe. Observed
//! live: a remediation agent, correctly, "made no edit because target_files
//! and ownership scopes are empty" — dispatched to fix a report it was
//! forbidden to touch.
//!
//! Deliverable contracts are host-parsed from the task files, not
//! agent-authored, so they are a trustworthy third channel: a root a contract
//! declares is a root the run may write. One guard keeps repository paths out:
//! a root whose directory exists under the target repository checkout is
//! repo-owned work and keeps the strict repository rules — only roots the repo
//! does not contain (which is what makes them project artifacts) are admitted.

use std::path::Path;

use crate::task_universe::WorkflowV2TaskUniverse;

/// Roots to admit, derived from every task's `deliverable_contracts`.
///
/// At most the first two path segments of each contract's directory — wide
/// enough to cover sibling deliverables in the same area, narrow enough that a
/// contract cannot claim the whole project.
pub(crate) fn contract_artifact_roots(
    universe: &WorkflowV2TaskUniverse,
    target_repository_root: Option<&str>,
) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for task in &universe.tasks {
        for contract in &task.deliverable_contracts {
            let Some(root) = root_of(&contract.artifact_path) else {
                continue;
            };
            if repo_owns(&root, target_repository_root) {
                continue;
            }
            if !roots.iter().any(|existing| *existing == root) {
                roots.push(root);
            }
        }
    }
    roots
}

/// The admissible root of one declared path, or `None` when the path cannot
/// safely contribute one: absolute, templated, traversing, or too shallow to
/// have a directory at all.
fn root_of(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("${")
        || trimmed.contains('*')
        || trimmed.contains('<')
        || Path::new(trimmed).is_absolute()
    {
        return None;
    }
    let segments: Vec<&str> = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.iter().any(|segment| *segment == "..") {
        return None;
    }
    // The last segment is the file; everything before it is the directory.
    let directory = &segments[..segments.len().saturating_sub(1)];
    if directory.is_empty() {
        return None;
    }
    Some(directory[..directory.len().min(2)].join("/"))
}

/// A root whose directory exists in the repository checkout is repository
/// work, and repository writes keep their strict declared-target rules.
fn repo_owns(root: &str, target_repository_root: Option<&str>) -> bool {
    target_repository_root.is_some_and(|repo| Path::new(repo).join(root).is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_universe::{WorkflowV2DeliverableContract, WorkflowV2TaskUniverseTask};

    fn universe_with(paths: &[&str]) -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-X-001".to_string(),
                deliverable_contracts: paths
                    .iter()
                    .map(|path| WorkflowV2DeliverableContract {
                        kind: "report".to_string(),
                        artifact_path: (*path).to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }
    }

    /// The live case: the contract declares a docs path the repo does not
    /// contain, so the run must be allowed to write it as a project artifact.
    #[test]
    fn a_contract_root_the_repo_lacks_is_admitted() {
        let repo = tempfile::tempdir().expect("repo");
        let universe = universe_with(&["docs/reports/audit.md"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert_eq!(roots, vec!["docs/reports".to_string()]);
    }

    /// A root that exists in the repository checkout is repository work and
    /// must NOT be widened into a project-artifact root.
    #[test]
    fn a_repo_owned_root_is_refused() {
        let repo = tempfile::tempdir().expect("repo");
        std::fs::create_dir_all(repo.path().join("src/lib")).expect("mkdir");
        let universe = universe_with(&["src/lib/module.rs"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert!(roots.is_empty(), "{roots:?}");
    }

    /// Templates, globs, traversal and bare filenames contribute nothing.
    #[test]
    fn unsafe_shapes_contribute_no_root() {
        let repo = tempfile::tempdir().expect("repo");
        let universe = universe_with(&[
            "${PROJECT_ROOT}/data/out.json",
            "reports/*.md",
            "../outside/file.md",
            "report.md",
        ]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert!(roots.is_empty(), "{roots:?}");
    }

    /// Deep contract paths widen to at most two segments — sibling files in
    /// the same area are covered without granting the whole tree.
    #[test]
    fn roots_are_capped_at_two_segments() {
        let repo = tempfile::tempdir().expect("repo");
        let universe = universe_with(&["out/reports/2026/q1/audit.md"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert_eq!(roots, vec!["out/reports".to_string()]);
    }
}
