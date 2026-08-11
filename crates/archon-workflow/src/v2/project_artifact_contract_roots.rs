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
//! agent-authored, so they are a trustworthy channel: a root a contract
//! declares is a root the run may write.
//!
//! One guard is essential. Contracts also declare source paths
//! (`crates/<crate>/src/<module>.rs`), and admitting those as artifact roots
//! would reclassify real code edits as artifacts — emptying `files_changed` and
//! silently disabling declared-target enforcement for every code task. So a
//! root the repository owns is refused and keeps the strict repository rules.
//!
//! Ownership is decided by **git tracking, not directory existence**. Testing
//! `is_dir()` conflates "a folder of this name is on disk" with "the repository
//! owns this path": a project deliverable at `docs/trading/` that is untracked
//! and gitignored in the code repo tripped that test purely because a same-named
//! directory happened to sit in both trees, and the deliverable it was meant to
//! admit was refused instead. Git answers the real question — `docs/trading` has
//! zero tracked files, `crates/archon-trading` has many.
//!
//! The check fails closed: if git cannot answer, the root is treated as
//! repository-owned. A wrong refusal blocks one deliverable; a wrong admission
//! disables write-ownership enforcement.

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
            if roots.iter().any(|existing| *existing == root) {
                continue;
            }
            if repository_tracks(&root, target_repository_root) {
                continue;
            }
            roots.push(root);
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

/// Does the repository track anything under this root?
///
/// Tracked content means the path is source the repository owns, so it keeps
/// the strict declared-target rules rather than becoming an artifact root. An
/// untracked or gitignored path is not repository content, whatever directories
/// happen to exist on disk.
///
/// Fails closed: a git invocation that cannot answer reports ownership, so an
/// unreadable repository refuses admission rather than widening write rights.
fn repository_tracks(root: &str, target_repository_root: Option<&str>) -> bool {
    let Some(repository) = target_repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    else {
        // No repository in play: nothing can claim the path as source.
        return false;
    };
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--", root])
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    !output.stdout.is_empty()
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

    /// A repository with `crates/thing/src/lib.rs` committed and an untracked,
    /// gitignored `docs/trading/` directory present on disk — the exact shape
    /// that produced the live failure.
    fn repository_with_tracked_code_and_untracked_docs() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("repo");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .output()
                .expect("git");
            assert!(status.status.success(), "git {args:?}");
        };
        run(&["init", "--quiet"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        std::fs::create_dir_all(repo.path().join("crates/thing/src")).expect("mkdir");
        std::fs::write(repo.path().join("crates/thing/src/lib.rs"), "// code\n").expect("write");
        std::fs::write(repo.path().join(".gitignore"), "/docs/*\n").expect("write");
        run(&["add", "crates", ".gitignore"]);
        run(&["commit", "--quiet", "-m", "code"]);
        // Present on disk, untracked and ignored: a project deliverable whose
        // directory name collides with the repository tree.
        std::fs::create_dir_all(repo.path().join("docs/trading")).expect("mkdir");
        std::fs::write(repo.path().join("docs/trading/audit.md"), "# audit\n").expect("write");
        repo
    }

    /// The live case: the deliverable's directory EXISTS in the code repository
    /// but is untracked and gitignored, so the repository does not own it and it
    /// must be admitted. Testing directory existence instead of tracking refused
    /// this root and blocked TASK-TDL-001 for an entire evening.
    #[test]
    fn an_untracked_docs_root_is_admitted_despite_the_directory_existing() {
        let repo = repository_with_tracked_code_and_untracked_docs();
        assert!(repo.path().join("docs/trading").is_dir(), "collision setup");

        let universe = universe_with(&["docs/trading/audit.md"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert_eq!(roots, vec!["docs/trading".to_string()]);
    }

    /// Contracts also declare source files. Admitting a tracked code root would
    /// reclassify real code edits as artifacts and silently disable
    /// declared-target enforcement for every code task — so it must be refused.
    #[test]
    fn a_tracked_code_root_is_refused() {
        let repo = repository_with_tracked_code_and_untracked_docs();
        let universe = universe_with(&["crates/thing/src/lib.rs"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert!(
            roots.is_empty(),
            "tracked source must not become a root: {roots:?}"
        );
    }

    /// A mixed universe keeps exactly the deliverable roots and drops the code.
    #[test]
    fn a_mixed_universe_admits_only_the_untracked_roots() {
        let repo = repository_with_tracked_code_and_untracked_docs();
        let universe = universe_with(&["crates/thing/src/lib.rs", "docs/trading/audit.md"]);
        let roots = contract_artifact_roots(&universe, Some(&repo.path().display().to_string()));
        assert_eq!(roots, vec!["docs/trading".to_string()]);
    }

    /// An unreadable repository fails closed: refusing admission blocks one
    /// deliverable, while admitting would disable ownership enforcement.
    #[test]
    fn an_unreadable_repository_refuses_admission() {
        let missing = tempfile::tempdir().expect("dir");
        let path = missing.path().join("not-a-repo");
        let universe = universe_with(&["docs/trading/audit.md"]);
        let roots = contract_artifact_roots(&universe, Some(&path.display().to_string()));
        assert!(roots.is_empty(), "must fail closed: {roots:?}");
    }

    /// Templates, globs, traversal and bare filenames contribute nothing.
    #[test]
    fn unsafe_shapes_contribute_no_root() {
        let universe = universe_with(&[
            "${PROJECT_ROOT}/data/out.json",
            "reports/*.md",
            "../outside/file.md",
            "report.md",
        ]);
        assert!(contract_artifact_roots(&universe, None).is_empty());
    }

    /// Deep contract paths widen to at most two segments — sibling files in
    /// the same area are covered without granting the whole tree.
    #[test]
    fn roots_are_capped_at_two_segments() {
        let universe = universe_with(&["out/reports/2026/q1/audit.md"]);
        assert_eq!(
            contract_artifact_roots(&universe, None),
            vec!["out/reports".to_string()]
        );
    }

    // ---- causal proof -----------------------------------------------------
    //
    // The tests above check this module in isolation. These drive the real
    // classification the live failure died in: an agent reports the deliverable
    // it wrote as a changed file under the project root, and the host must
    // reclassify it as a project artifact. If it stays in `files_changed`, the
    // write-ownership check sees a changed file against empty target_files and
    // rejects the branch with "declares no target ownership" — the exact live
    // error. A negative control asserts the contract root is what makes the
    // difference, so a future regression cannot pass by coincidence.

    use crate::v2::project_artifacts::{
        WorkflowV2ProjectArtifactContext, normalize_project_artifact_files,
    };
    use crate::v2::result::{WorkflowV2FileRecord, WorkflowV2Result};

    /// A project root holding the deliverable, returned canonicalized so path
    /// prefix-stripping matches on macOS (`/var` vs `/private/var`).
    fn project_root_with_deliverable() -> (tempfile::TempDir, String) {
        let project = tempfile::tempdir().expect("project");
        let canonical = project.path().canonicalize().expect("canonicalize");
        std::fs::create_dir_all(canonical.join("docs/trading")).expect("mkdir");
        std::fs::write(canonical.join("docs/trading/audit.md"), "# audit\n").expect("write");
        let root = canonical.display().to_string();
        (project, root)
    }

    fn result_reporting_changed(path: &str) -> WorkflowV2Result {
        let mut result = WorkflowV2Result::accepted("wrote the deliverable");
        result.files_changed = vec![WorkflowV2FileRecord::new(path)];
        result
    }

    #[test]
    fn the_contract_root_reclassifies_the_deliverable_out_of_changed_files() {
        let repo = repository_with_tracked_code_and_untracked_docs();
        let (_project, project_root) = project_root_with_deliverable();
        let written = format!("{project_root}/docs/trading/audit.md");

        let mut context = WorkflowV2ProjectArtifactContext {
            project_root: Some(project_root),
            ..Default::default()
        };
        context.add_contract_roots(
            &universe_with(&["docs/trading/audit.md"]),
            Some(&repo.path().display().to_string()),
        );
        assert!(
            context.artifact_roots.contains(&"docs/trading".to_string()),
            "precondition: the contract root must be admitted: {:?}",
            context.artifact_roots
        );

        let mut result = result_reporting_changed(&written);
        normalize_project_artifact_files("inventory-tdl-001", &mut result, &context)
            .expect("classification");

        assert!(
            result.files_changed.is_empty(),
            "the deliverable must leave files_changed, or write-ownership rejects it: {:?}",
            result.files_changed
        );
        assert_eq!(result.artifacts.len(), 1, "it must become a project artifact");
    }

    /// Negative control: without the contract root the same file stays a
    /// changed file — reproducing the live rejection, and proving the root is
    /// the operative difference rather than something incidental.
    #[test]
    fn without_the_contract_root_the_deliverable_stays_a_changed_file() {
        let (_project, project_root) = project_root_with_deliverable();
        let written = format!("{project_root}/docs/trading/audit.md");

        let context = WorkflowV2ProjectArtifactContext {
            project_root: Some(project_root),
            ..Default::default()
        };

        let mut result = result_reporting_changed(&written);
        normalize_project_artifact_files("inventory-tdl-001", &mut result, &context)
            .expect("classification");

        assert_eq!(
            result.files_changed.len(),
            1,
            "control: an unadmitted path must remain a repository change"
        );
    }
}
