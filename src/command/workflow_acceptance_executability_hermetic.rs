//! A hermetic copy of the tree at one commit, for a probe with no scratch
//! policy (A4).
//!
//! The acceptance stage without `[workflow.acceptance_execution]` runs its
//! checks in the live project and repository; a probe must never do that
//! before or outside a round, so it builds what the scratch observation
//! builds: a clone of the repository at the commit (sharing the source's
//! objects, writing nothing into it) and, for the project,
//!
//! - its tracked copy inside that clone when the project lives inside the
//!   repository and is tracked at the commit, or
//! - otherwise (a project outside the repository, or one the commit does not
//!   track) a copy of the project's own data beside the clone: every regular
//!   file except what the scratch observation never exports either
//!   (credentials and engine configuration, run state under
//!   `.archon/workflows`, git metadata), the build output directory
//!   `target`, and the repository itself when it is nested in the project.
//!
//! The copy is made off the async runtime (`spawn_blocking`). Its project
//! data is digested before and after copying: data that changed while it
//! was copied is no evidence of any tree, and the run is unproven. A copy
//! larger than [`PROJECT_COPY_BYTES`] is refused with the reason, never
//! truncated. Checks run under the acceptance stage's own direct-site
//! environment (the host environment with the engine's credentials withheld,
//! `archon_tools::bash_env`), building warm from one cached target directory
//! per repository under the copy parent. Copies older than a day that a
//! killed probe left are swept before a new one is made; a copy that cannot
//! be removed is reported.

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::{CHECK_DEFERRED, CHECK_TIMED_OUT, CheckAllowance};

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
pub(super) struct Unrun(pub(super) String);

/// The acceptance direct site's environment (`archon_tools::bash::host_env`,
/// as `workflow_live_v3_acceptance_checks` builds it: the host's, with the
/// engine's own credentials withheld), with the build directory set to
/// `target`: a probe never builds into the host's own target directory.
pub(super) fn probe_environment(target: Option<&Path>) -> BTreeMap<String, String> {
    with_target(archon_tools::bash::host_env(), target)
}

pub(super) fn with_target(
    vars: impl IntoIterator<Item = (String, String)>,
    target: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut environment: BTreeMap<String, String> = vars.into_iter().collect();
    environment.remove("CARGO_TARGET_DIR");
    if let Some(target) = target {
        environment.insert(
            "CARGO_TARGET_DIR".into(),
            target.to_string_lossy().into_owned(),
        );
    }
    environment
}

/// Project paths never copied, as the scratch observation never exports
/// them (`acceptance_scratch_inputs::excluded`), plus the build output the
/// scratch refuses to overlay.
fn excluded(relative: &Path) -> bool {
    matches!(
        relative.to_str(),
        Some(
            "credentials"
                | "credentials.toml"
                | "config.toml"
                | "config.json"
                | ".env"
                | ".archon/config.toml"
                | "target"
        )
    ) || relative.starts_with(".archon/workflows")
        || relative.starts_with(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        || relative.components().any(|part| part.as_os_str() == ".git")
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
    let output = std::process::Command::new("git")
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

/// A digest of the project data a copy takes: every copied path, its size
/// and modification time.
pub(super) fn data_digest(project: &Path, repository: &Path) -> String {
    let mut lines = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        if !relative.as_os_str().is_empty() && excluded(&relative) {
            continue;
        }
        let path = project.join(&relative);
        if path == repository {
            continue;
        }
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.is_dir() {
            for item in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                stack.push(relative.join(item.file_name()));
            }
        } else if meta.is_file() {
            let modified = (meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());
            lines.push(format!(
                "{}\t{}\t{modified}",
                relative.display(),
                meta.len()
            ));
        }
    }
    lines.sort();
    content_digest(lines.join("\n").as_bytes())
}

impl HermeticCopy {
    /// `repository` at `commit` and `project`'s data, under `parent`.
    fn at(parent: &Path, repository: &Path, project: &Path, commit: &str) -> Result<Self, Unrun> {
        let canonical = |path: &Path| {
            path.canonicalize()
                .map(archon_shell::paths::plain)
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
                let before = data_digest(&live_project, &source);
                let mut remaining = PROJECT_COPY_BYTES;
                copy_data(
                    &live_project,
                    Path::new(""),
                    &copy.project,
                    &source,
                    &mut remaining,
                )?;
                if data_digest(&live_project, &source) != before {
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
    remaining: &mut u64,
) -> Result<(), Unrun> {
    let io = |path: &Path, error: std::io::Error| Unrun(format!("{}: {error}", path.display()));
    if !relative.as_os_str().is_empty() && excluded(relative) {
        return Ok(());
    }
    let source = root.join(relative);
    if source == repository {
        return Ok(());
    }
    let meta = source.symlink_metadata().map_err(|e| io(&source, e))?;
    let target = dest.join(relative);
    if meta.is_dir() {
        std::fs::create_dir_all(&target).map_err(|e| io(&target, e))?;
        for item in std::fs::read_dir(&source).map_err(|e| io(&source, e))? {
            let item = item.map_err(|e| io(&source, e))?;
            copy_data(
                root,
                &relative.join(item.file_name()),
                dest,
                repository,
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
        (repository.canonicalize().map(archon_shell::paths::plain))
            .unwrap_or_else(|_| repository.to_path_buf())
            .to_string_lossy()
            .as_bytes(),
    );
    parent.join(WARM_TARGETS).join(&key[..16])
}

fn deferred(id: &str) -> CheckResult {
    CheckResult {
        acceptance_id: id.to_string(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        operational_error: Some(CHECK_DEFERRED.to_string()),
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
    let (repo, project, at) = (
        repository.to_path_buf(),
        probe.project.clone(),
        commit.to_string(),
    );
    let copy = tokio::task::spawn_blocking(move || HermeticCopy::at(&parent, &repo, &project, &at))
        .await
        .map_err(|error| Unrun(format!("the copy task failed: {error}")))??;
    let target = warm_target(&probe.copy_parent, repository);
    let mut site = DirectSite {
        repository: copy.repository.clone(),
        project: copy.project.clone(),
        environment: probe_environment(Some(&target)),
        timeout_secs: DIRECT_DEFAULT_TIMEOUT_SECS,
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
                site.timeout_secs = timeout_secs.clamp(1, DIRECT_DEFAULT_TIMEOUT_SECS);
                cut && site.timeout_secs < DIRECT_DEFAULT_TIMEOUT_SECS
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
