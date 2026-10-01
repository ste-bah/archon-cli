//! The roots the review tripwire watches, derived from the run's own records
//! and nothing else: no directory name is written here.
//!
//! Watched, wherever each lives on disk:
//! - the checkout, ignored files included;
//! - the project root;
//! - the project artifact roots the host hands every agent;
//! - every artifact, deliverable, registry and instance-source path the task
//!   set declares, and every file a task declares it changes, as the
//!   directory it resolves to (absolute as given; relative against the
//!   project root and the checkout);
//! - the acceptance policy's project inputs.
//!
//! Not watched: the checkout's git directory, the build's target directories
//! (`CARGO_TARGET_DIR`, and `cargo metadata` for a checkout that has a
//! manifest), the host's toolchain directories (`SHARED_TOOLCHAIN_DIRS`), the
//! run stores (the host writes them while a review runs), the acceptance
//! scratch parent and the host's own stores. A root inside one of those is
//! still watched (an exclude only hides what it contains from a wider root).

use std::path::{Path, PathBuf};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::write_coordinator::{fixture_provenance, sealed_roots};

use super::review_tree::{WatchSet, git};

fn is_glob_part(part: &str) -> bool {
    part.contains(['*', '?', '[', '<', '{'])
}

/// `declared` up to its first wildcard component, resolved against each of
/// `bases` (or as given when absolute); the directory each names.
fn declared_dirs(declared: &str, bases: &[&Path]) -> Vec<PathBuf> {
    let declared = declared.trim();
    if declared.is_empty() {
        return Vec::new();
    }
    let literal: PathBuf = Path::new(declared)
        .components()
        .take_while(|part| !is_glob_part(&part.as_os_str().to_string_lossy()))
        .collect();
    if literal.as_os_str().is_empty() {
        return Vec::new();
    }
    let resolved: Vec<PathBuf> = if literal.is_absolute() {
        vec![literal]
    } else {
        bases.iter().map(|base| base.join(&literal)).collect()
    };
    resolved
        .into_iter()
        .filter_map(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            }
        })
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

/// The build's target directories for the checkout at `repo`.
fn target_dirs(repo: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|dir| {
            if dir.is_absolute() {
                dir
            } else {
                repo.join(dir)
            }
        })
        .into_iter()
        .collect();
    if repo.join("Cargo.toml").is_file()
        && let Ok(output) = std::process::Command::new("cargo")
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .current_dir(repo)
            .output()
        && output.status.success()
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        && let Some(dir) = value.get("target_directory").and_then(|dir| dir.as_str())
    {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

/// The watch set for a review map of the run whose store is at `run_root`.
pub(super) fn review_roots(
    run_root: &Path,
    project: Option<&Path>,
    artifact_roots: &[String],
    repo: Option<&Path>,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> WatchSet {
    let bases: Vec<&Path> = project.into_iter().chain(repo).collect();
    let mut roots: Vec<PathBuf> = bases.iter().map(|base| base.to_path_buf()).collect();
    if let Some(project) = project {
        roots.extend(artifact_roots.iter().map(|root| project.join(root)));
    }
    for task in universe.map(|u| u.tasks.as_slice()).unwrap_or_default() {
        let declared = (task.artifact_requirements.iter())
            .chain(task.files_expected_to_change.iter())
            .map(String::as_str)
            .chain(task.deliverable_contracts.iter().flat_map(|contract| {
                [
                    Some(contract.artifact_path.as_str()),
                    contract.registry_path.as_deref(),
                    contract.instance_source_path.as_deref(),
                ]
                .into_iter()
                .flatten()
            }));
        for path in declared {
            roots.extend(declared_dirs(path, &bases));
        }
    }
    for input in fixture_provenance::project_inputs_of(run_root) {
        roots.extend(match (input.is_absolute(), project) {
            (true, _) => Some(input),
            (false, Some(project)) => Some(project.join(input)),
            (false, None) => None,
        });
    }
    let mut excludes: Vec<PathBuf> = vec![run_root.to_path_buf()];
    // Every run's store sits beside this one; a project child holding them
    // is the host's engine directory, never project content.
    if let Some(stores) = run_root.parent() {
        excludes.push(stores.to_path_buf());
        if let Some(project) = project
            && let Ok(inside) = run_root.strip_prefix(project)
            && let Some(first) = inside.components().next()
        {
            excludes.push(project.join(first));
        }
    }
    excludes.extend(
        sealed_roots::recorded_policy_roots(run_root)
            .into_iter()
            .filter(|root| Some(root.as_path()) != project && Some(root.as_path()) != repo),
    );
    excludes.extend(sealed_roots::user_host_stores());
    if let Some(repo) = repo {
        excludes.push(repo.join(".git"));
        if let Some(dir) = git(repo, &["rev-parse", "--absolute-git-dir"]) {
            excludes.push(PathBuf::from(String::from_utf8_lossy(&dir).trim()));
        }
        excludes.extend(target_dirs(repo));
    }
    roots.retain(|root| root.is_absolute());
    roots.sort();
    roots.dedup();
    excludes.sort();
    excludes.dedup();
    // A root a wider root already walks, with nothing between them hiding
    // it, is walked once.
    let covered = |root: &PathBuf| {
        roots.iter().any(|wider| {
            wider != root
                && root.starts_with(wider)
                && !excludes
                    .iter()
                    .any(|exclude| exclude.starts_with(wider) && root.starts_with(exclude))
        })
    };
    let kept: Vec<PathBuf> = roots
        .iter()
        .filter(|root| !covered(root))
        .cloned()
        .collect();
    WatchSet {
        repo: repo.map(Path::to_path_buf),
        roots: kept,
        excludes,
    }
}
