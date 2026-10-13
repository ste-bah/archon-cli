//! A hermetic copy of one committed tree for a probe without scratch policy.
//!
//! The acceptance stage without `[workflow.acceptance_execution]` runs its
//! checks in the live project and repository, so it builds a clone of the
//! repository at the commit and a project copy matching scratch observation:
//!
//! - its tracked copy inside the clone, or
//! - otherwise a copy beside the clone of regular project files, excluding
//!   credentials, engine configuration, harness run state, git metadata,
//!   `target`, and any nested repository.
//!
//! The copy is made off the async runtime. Project data is digested before
//! and after copying; changes make the run unproven, and oversized copies are
//! refused. Checks use the direct-site environment and a cached target under
//! the copy parent. Stale copies are swept; removal failures are reported.

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::{CHECK_DEFERRED, CHECK_TIMED_OUT, CheckAllowance};
use archon_workflow::acceptance_scratch::{acceptance_input_excluded, relative_key};
use archon_workflow::acceptance_scratch::{nested_repository, nested_repository_skipped};

use super::*;

/// The largest project copy a probe makes without a scratch policy naming
/// what the checks read.
pub(super) const PROJECT_COPY_BYTES: u64 = 16 << 30;
/// Copies a probe leaves are swept once they are this old.
const STALE_COPY_SECS: u64 = 24 * 60 * 60;
const COPY_PREFIX: &str = "archon-probe-";
/// The warm build directories, one per repository, under the copy parent.
pub(super) const WARM_TARGETS: &str = "archon-probe-targets";

/// Why a hermetic run gave no verdict.
#[derive(Debug)]
pub(super) struct Unrun(pub(super) String);

impl HostProbe {
    /// Make the probe's own hermetic copies under `parent`.
    #[cfg(all(test, unix))]
    pub(crate) fn with_copy_parent(mut self, parent: PathBuf) -> Self {
        self.copy_parent = parent;
        self
    }
}

fn excluded(relative: &Path, task_root: Option<&Path>) -> bool {
    acceptance_input_excluded(relative, &[], task_root)
}

/// The copy. [`HermeticCopy::remove`] removes it and says whether it could;
/// dropping it removes it best-effort.
pub(super) struct HermeticCopy {
    root: PathBuf,
    pub(super) repository: PathBuf,
    pub(super) project: PathBuf,
    removed: bool,
}

impl HermeticCopy {
    pub(super) fn remove(mut self) -> Result<(), String> {
        self.removed = true;
        std::fs::remove_dir_all(&self.root)
            .map_err(|error| format!("{}: {error}", self.root.display()))
    }
}

impl Drop for HermeticCopy {
    fn drop(&mut self) {
        if !self.removed {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn git(args: &[&std::ffi::OsStr]) -> Result<(), Unrun> {
    let output = archon_shell::spawn::command("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .map_err(|error| Unrun(format!("git: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(Unrun(format!(
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
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let stale = (entry.metadata().ok())
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age.as_secs() > STALE_COPY_SECS);
        if name.to_string_lossy().starts_with(COPY_PREFIX)
            && name != WARM_TARGETS
            && stale
            && let Err(error) = std::fs::remove_dir_all(entry.path())
        {
            failures.push(format!("{}: {error}", entry.path().display()));
        }
    }
    failures
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
        Unrun(format!(
            "could not canonicalize project root {}: {error}",
            project.display()
        ))
    })?;
    let repository = archon_shell::paths::canonicalize(repository).map_err(|error| {
        Unrun(format!(
            "could not canonicalize repository root {}: {error}",
            repository.display()
        ))
    })?;
    let task_root_relative = task_root_relative(&project, task_root)?;
    // Issue 440: the one nested-repository rule the copies apply.
    let nested = nested_repository(&project, &repository);
    let mut lines = Vec::new();
    let mut stack = vec![scope.to_path_buf()];
    while let Some(relative) = stack.pop() {
        if !relative.as_os_str().is_empty()
            && (excluded(&relative, task_root_relative.as_deref())
                || acceptance_input_excluded(&relative, excludes, task_root_relative.as_deref()))
        {
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
                Unrun(format!(
                    "could not canonicalize task root {}: {error}",
                    root.display()
                ))
            })?;
            task_root_relative_paths(project, &canonical).map_err(|_| {
                Unrun(format!(
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

impl HermeticCopy {
    /// `repository` at `commit` and `project`'s data, under `parent`.
    fn at(
        parent: &Path,
        repository: &Path,
        project: &Path,
        task_root: Option<&Path>,
        commit: &str,
    ) -> Result<Self, Unrun> {
        let canonical = |path: &Path| {
            archon_shell::paths::canonicalize(path)
                .map_err(|error| Unrun(format!("{}: {error}", path.display())))
        };
        let (source, live_project) = (canonical(repository)?, canonical(project)?);
        std::fs::create_dir_all(parent)
            .map_err(|error| Unrun(format!("{}: {error}", parent.display())))?;
        let root = canonical(parent)?.join(format!("{COPY_PREFIX}{}", uuid::Uuid::new_v4()));
        for live in [&source, &live_project] {
            if root.starts_with(live) {
                return Err(Unrun(format!(
                    "the probe copy parent {} lies inside the live root {}",
                    parent.display(),
                    live.display()
                )));
            }
        }
        let mut copy = Self {
            repository: root.join("repo"),
            project: root.join("project"),
            root,
            removed: false,
        };
        git(&[
            "clone".as_ref(),
            "--quiet".as_ref(),
            "--shared".as_ref(),
            "--no-checkout".as_ref(),
            "--".as_ref(),
            source.as_os_str(),
            copy.repository.as_os_str(),
        ])?;
        git(&[
            "-C".as_ref(),
            copy.repository.as_os_str(),
            "checkout".as_ref(),
            "--quiet".as_ref(),
            "--detach".as_ref(),
            commit.as_ref(),
            "--".as_ref(),
        ])?;
        let tracked = live_project
            .strip_prefix(&source)
            .ok()
            .map(|relative| copy.repository.join(relative))
            .filter(|inside| inside.is_dir());
        match tracked {
            Some(inside) => copy.project = inside,
            None => {
                let task_root = task_root.map(canonical).transpose()?;
                let task_root_relative = task_root
                    .as_deref()
                    .and_then(|root| root.strip_prefix(&live_project).ok());
                let before = data_digest(&live_project, &source, task_root.as_deref())?;
                let mut remaining = PROJECT_COPY_BYTES;
                copy_data(
                    &live_project,
                    Path::new(""),
                    &copy.project,
                    &source,
                    task_root_relative,
                    &mut remaining,
                )?;
                if data_digest(&live_project, &source, task_root.as_deref())? != before {
                    return Err(Unrun(
                        "the project data changed while it was copied, so the copy is no evidence of any one tree".into(),
                    ));
                }
            }
        }
        Ok(copy)
    }
}

/// Copy `root/relative` into `dest/relative`: regular files and
/// directories only (a symlink is never followed), skipping [`excluded`]
/// paths and the nested `repository`.
fn copy_data(
    root: &Path,
    relative: &Path,
    dest: &Path,
    repository: &Path,
    task_root: Option<&Path>,
    remaining: &mut u64,
) -> Result<(), Unrun> {
    let root = archon_shell::paths::canonicalize(root).map_err(|error| {
        Unrun(format!(
            "could not canonicalize project root {}: {error}",
            root.display()
        ))
    })?;
    let task_root = task_root_relative(&root, task_root)?;
    let repository = archon_shell::paths::canonicalize(repository)
        .map_err(|error| Unrun(format!("{}: {error}", repository.display())))?;
    copy_data_relative(
        &root,
        relative,
        dest,
        nested_repository(&root, &repository).as_deref(),
        task_root.as_deref(),
        remaining,
    )
}

fn copy_data_relative(
    root: &Path,
    relative: &Path,
    dest: &Path,
    nested: Option<&Path>,
    task_root: Option<&Path>,
    remaining: &mut u64,
) -> Result<(), Unrun> {
    let io = |path: &Path, error: std::io::Error| Unrun(format!("{}: {error}", path.display()));
    let source = root.join(relative);
    if !relative.as_os_str().is_empty() && excluded(relative, task_root) {
        return Ok(());
    }
    if nested_repository_skipped(relative, nested) {
        return Ok(());
    }
    let meta = source.symlink_metadata().map_err(|e| io(&source, e))?;
    let target = dest.join(relative);
    if meta.is_dir() {
        std::fs::create_dir_all(&target).map_err(|e| io(&target, e))?;
        for item in std::fs::read_dir(&source).map_err(|e| io(&source, e))? {
            let item = item.map_err(|e| io(&source, e))?;
            copy_data_relative(
                root,
                &relative.join(item.file_name()),
                dest,
                nested,
                task_root,
                remaining,
            )?;
        }
    } else if meta.is_file() {
        if meta.len() > *remaining {
            return Err(Unrun(format!(
                "the project data exceeds the {} GiB a probe copies without a scratch policy; configure [workflow.acceptance_execution] with the project_inputs the checks read",
                PROJECT_COPY_BYTES >> 30
            )));
        }
        *remaining -= meta.len();
        std::fs::copy(&source, &target).map_err(|e| io(&source, e))?;
    }
    Ok(())
}

/// The warm build directory for `repository`'s copies.
pub(super) fn warm_target(parent: &Path, repository: &Path) -> PathBuf {
    let key = content_digest(
        (archon_shell::paths::canonicalize(repository))
            .unwrap_or_else(|_| repository.to_path_buf())
            .to_string_lossy()
            .as_bytes(),
    );
    parent.join(WARM_TARGETS).join(&key[..16])
}

fn deferred(id: &str) -> CheckResult {
    CheckResult {
        classification: None,
        acceptance_id: id.to_string(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        operational_error: Some(CHECK_DEFERRED.to_string()),
        environment_note: None,
    }
}

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
) -> Result<Vec<CheckResult>, Unrun> {
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
    .map_err(|error| Unrun(format!("the copy task failed: {error}")))??;
    let target = warm_target(&probe.copy_parent, repository);
    let mut site = DirectSite {
        repository: copy.repository.clone(),
        project: copy.project.clone(),
        // The direct site's default policy; a probe never builds into the
        // host's own target directory.
        host: probe.host.clone(),
        policy: None,
        target: Some(target),
        fresh_home: true,
        // Issue 323: the probe's one per-check bound, as at every site.
        timeout_secs: probe.check_bound_secs(),
        output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut results = Vec::new();
    let mut failure = None;
    for (index, reference) in refs.iter().enumerate() {
        // Issue 255: the freeze's budget bounds each check, as the scratch
        // observation's hooks do there.
        let allowance = hooks.allowance.as_ref().map(|allowance| allowance());
        let cut = match allowance {
            Some(CheckAllowance::Defer) => {
                results.extend(refs[index..].iter().map(|r| deferred(&r.acceptance_id)));
                break;
            }
            Some(CheckAllowance::Run { timeout_secs, cut }) => {
                site.timeout_secs = timeout_secs.clamp(1, probe.check_bound_secs());
                cut
            }
            None => false,
        };
        match run_check_direct(&site, contract, digest, reference, cancel.clone()).await {
            Ok(mut result) => {
                let stopped = cut && result.operational_error.as_deref() == Some(CHECK_TIMED_OUT);
                if stopped {
                    result.operational_error = Some(CHECK_DEFERRED.to_string());
                }
                if let Some(on_check) = &hooks.on_check {
                    on_check(&result);
                }
                results.push(result);
                if stopped {
                    results.extend(refs[index + 1..].iter().map(|r| deferred(&r.acceptance_id)));
                    break;
                }
            }
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
        }
    }
    let removal = tokio::task::spawn_blocking(move || copy.remove()).await;
    match removal {
        Ok(Ok(())) => {}
        Ok(Err(error)) => probe.note(format!("a probe copy could not be removed: {error}")),
        Err(error) => probe.note(format!("a probe copy's removal failed: {error}")),
    }
    match failure {
        Some(error) => Err(Unrun(error)),
        None => Ok(results),
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_hermetic_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "workflow_acceptance_executability_nested_repository_tests.rs"]
mod nested_repository_tests;
