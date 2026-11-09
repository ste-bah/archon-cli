//! Hermetic committed-tree copies for probes without a scratch policy.
//! Repository and project inputs are copied off the async runtime and
//! digested before and after copying; changes make the run unproven.
//! Checks use the direct-site environment. Stale copies are swept and
//! removal failures are reported.

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::{CHECK_DEFERRED, CHECK_TIMED_OUT, CheckAllowance};
use archon_workflow::acceptance_scratch::{acceptance_input_excluded, relative_key};
use archon_workflow::acceptance_scratch::{nested_repository, nested_repository_skipped};

use super::*;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_offline_replay_tests.rs"]
mod offline_replay_tests;

#[path = "workflow_acceptance_executability_siblings.rs"]
pub(super) mod siblings;

#[path = "workflow_acceptance_executability_hermetic_layout.rs"]
mod layout;
pub(crate) use layout::{
    CheckEvidence, CopyLayout, CopyRun, SiblingRevision, deferred, warm_target,
};

pub(super) const PROJECT_COPY_BYTES: u64 = 16 << 30;
const STALE_COPY_SECS: u64 = 24 * 60 * 60;
const COPY_PREFIX: &str = "archon-probe-";
/// The warm build directories, one per repository, under the copy parent.
pub(super) const WARM_TARGETS: &str = "archon-probe-targets";

/// Why a hermetic run gave no verdict.
pub(super) use crate::command::workflow_host_probe_fault::Unrun;

impl HostProbe {
    /// Make the probe's own hermetic copies under `parent`.
    #[cfg(all(test, unix))]
    pub(crate) fn with_copy_parent(mut self, parent: PathBuf) -> Self {
        self.copy_parent = parent;
        self
    }
}

/// The copy. [`HermeticCopy::remove`] removes it and says whether it could;
/// dropping it removes it best-effort.
pub(crate) struct HermeticCopy {
    root: PathBuf,
    pub(super) repository: PathBuf,
    pub(super) project: PathBuf,
    removed: bool,
    base_guard: Option<base_copy::BaseGuard>,
    isolated_git: Option<base_git::IsolatedGit>,
    prefix: PathBuf,
    modified: Mutex<BTreeMap<String, std::time::SystemTime>>,
}

impl HermeticCopy {
    fn layout(&self) -> CopyLayout {
        CopyLayout {
            root: self.root.clone(),
            repository: self.repository.clone(),
            site_repository: Some(self.repository.clone()),
            project: self.project.clone(),
            siblings: Vec::new(),
            check_siblings: std::collections::BTreeMap::new(),
            site_build_cache: None,
        }
    }
}

impl HermeticCopy {
    pub(super) fn remove(mut self) -> Result<(), String> {
        self.removed = true;
        owner::remove(&self.root)
    }
}

impl Drop for HermeticCopy {
    fn drop(&mut self) {
        if !self.removed {
            let _ = owner::remove_now(&self.root);
        }
    }
}

fn git(args: &[&std::ffi::OsStr]) -> Result<(), Unrun> {
    let output = archon_shell::spawn::command("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .map_err(|error| {
            let text = format!("git: {error}");
            Unrun::error(error.into(), text)
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(Unrun::unknown(format!(
        "git {}: {}",
        args.iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Remove copies a killed probe left under `parent`; what cannot be removed
/// is returned for the caller to report.
pub(super) fn sweep(parent: &Path) -> Vec<String> {
    owner::sweep(parent)
}

/// A content digest of the project data a copy takes.
pub(super) fn data_digest(
    project: &Path,
    repository: &Path,
    task_root: Option<&Path>,
) -> Result<String, Unrun> {
    data_digest_scoped(project, repository, task_root, Path::new(""), &[])
}

pub(super) fn data_digest_scoped(
    project: &Path,
    repository: &Path,
    task_root: Option<&Path>,
    scope: &Path,
    excludes: &[PathBuf],
) -> Result<String, Unrun> {
    let project = archon_shell::paths::canonicalize(project).map_err(|error| {
        let text = format!(
            "could not canonicalize project root {}: {error}",
            project.display()
        );
        Unrun::error(error.into(), text)
    })?;
    let repository = archon_shell::paths::canonicalize(repository).map_err(|error| {
        let text = format!(
            "could not canonicalize repository root {}: {error}",
            repository.display()
        );
        Unrun::error(error.into(), text)
    })?;
    let task_root_relative = task_root_relative(&project, task_root)?;
    let nested = nested_repository(&project, &repository);
    let mut lines = Vec::new();
    let mut stack = vec![scope.to_path_buf()];
    while let Some(relative) = stack.pop() {
        if acceptance_input_excluded(&relative, excludes, task_root_relative.as_deref()) {
            continue;
        }
        if nested_repository_skipped(&relative, nested.as_deref()) {
            continue;
        }
        let path = project.join(&relative);
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.is_dir() {
            for item in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                stack.push(relative.join(item.file_name()));
            }
        } else if meta.is_file() {
            let digest = std::fs::read(&path)
                .map(|bytes| content_digest(&bytes))
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
            lines.push(format!("{}\t{digest}", relative_key(&relative)));
        }
    }
    lines.sort();
    Ok(content_digest(lines.join("\n").as_bytes()))
}

/// Normalize the task root to the project-relative paths used by the shared
/// input exclusion predicate. Reject roots outside the project explicitly.
fn task_root_relative(project: &Path, task_root: Option<&Path>) -> Result<Option<PathBuf>, Unrun> {
    task_root
        .map(|root| {
            let rooted = if root.is_absolute() {
                root.to_path_buf()
            } else {
                project.join(root)
            };
            let canonical = archon_shell::paths::canonicalize(&rooted).map_err(|error| {
                let text = format!(
                    "could not canonicalize task root {}: {error}",
                    root.display()
                );
                Unrun::error(error.into(), text)
            })?;
            task_root_relative_paths(project, &canonical).map_err(|_| {
                Unrun::unknown(format!(
                    "task root {} is outside project {}",
                    canonical.display(),
                    project.display()
                ))
            })
        })
        .transpose()
}

fn task_root_relative_paths(project: &Path, task_root: &Path) -> Result<PathBuf, ()> {
    let project = archon_shell::paths::plain(project.to_path_buf());
    let task_root = archon_shell::paths::plain(task_root.to_path_buf());
    task_root
        .strip_prefix(&project)
        .map(Path::to_path_buf)
        .map_err(|_| ())
}

#[allow(dead_code)]
#[path = "workflow_acceptance_executability_breakage.rs"]
pub(crate) mod breakage;
#[path = "workflow_acceptance_executability_breakage_owner.rs"]
mod owner;
#[allow(unused_imports)]
pub(crate) use breakage::{BreakageReport, BreakageRequest, MIN_DEADLINE_SECS, prove_breakage};
#[path = "workflow_acceptance_executability_base_git.rs"]
mod base_git;
#[path = "workflow_acceptance_breaker_view.rs"]
pub(crate) mod breaker_view;

#[path = "workflow_acceptance_executability_copy_create.rs"]
mod create;

#[allow(dead_code)]
#[path = "workflow_acceptance_executability_base_copy.rs"]
pub(crate) mod base_copy;
#[allow(unused_imports)]
pub(crate) use base_copy::{BaseCopy, materialise_base_copy};

/// Copy `root/relative` into `dest/relative`: regular files and
/// directories only (a symlink is never followed), skipping excluded
/// paths and the nested `repository`.
#[path = "workflow_acceptance_executability_copy_data.rs"]
mod copy_data_impl;
#[cfg(test)]
use copy_data_impl::copy_data;
use copy_data_impl::copy_data_scoped;

/// Run `refs` of `contract` in one hermetic copy of `repository` at
/// `commit` (see the module docs).
pub(super) async fn run_in_copy(
    probe: &HostProbe,
    repository: &Path,
    commit: &str,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    hooks: &archon_workflow::acceptance_scratch::ObserveHooks,
    observed_layout: &std::sync::Mutex<Option<CopyLayout>>,
) -> Result<CopyRun, Unrun> {
    let parent = probe.copy_parent.clone();
    for failure in sweep(&parent) {
        probe.note(format!(
            "a stale probe copy could not be removed: {failure}"
        ));
    }
    #[cfg(test)]
    probe
        .copies_made
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (repo, project, task_root, at) = (
        repository.to_path_buf(),
        probe.project.clone(),
        probe.task_root.clone(),
        commit.to_string(),
    );
    let copy = tokio::task::spawn_blocking(move || {
        HermeticCopy::at(&parent, &repo, &project, task_root.as_deref(), &at)
    })
    .await
    .map_err(|error| {
        let text = format!("the copy task failed: {error}");
        Unrun::error(error.into(), text)
    })??;
    let result = run_refs_in_copy(
        probe,
        &copy,
        repository,
        contract,
        digest,
        refs,
        hooks,
        observed_layout,
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    let removal = tokio::task::spawn_blocking(move || copy.remove()).await;
    match removal {
        Ok(Ok(())) => {}
        Ok(Err(error)) => probe.note(format!("a probe copy could not be removed: {error}")),
        Err(error) => probe.note(format!("a probe copy's removal failed: {error}")),
    }
    result
}
#[path = "workflow_acceptance_executability_copy_run.rs"]
mod run;
#[allow(unused_imports)]
pub(crate) use run::run_refs_in_copy;

#[cfg(test)]
#[path = "workflow_acceptance_executability_nested_repository_tests.rs"]
mod nested_repository_tests;
#[cfg(test)]
#[path = "workflow_acceptance_executability_hermetic_tests.rs"]
mod tests;
