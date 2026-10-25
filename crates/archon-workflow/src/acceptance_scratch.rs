//! Native observation roots. Construction and audit, not a hostile-code sandbox.
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[path = "acceptance_scratch_cache.rs"]
mod cache;
#[path = "acceptance_scratch_control.rs"]
mod control;
#[path = "acceptance_scratch_direct.rs"]
mod direct;
pub use direct::{
    DIRECT_DEFAULT_OUTPUT_BYTES, DIRECT_DEFAULT_TIMEOUT_SECS, DirectSite, evaluate_floor_direct,
    run_check_direct, run_check_direct_with_plan,
};
#[path = "acceptance_scratch_inputs.rs"]
mod inputs;
#[path = "acceptance_scratch_siblings.rs"]
mod siblings;
#[path = "acceptance_scratch_site_path.rs"]
mod site_path;
use control::git;

#[path = "acceptance_scratch_paths.rs"]
mod paths;
pub use paths::relative_key;
#[cfg(test)]
pub(crate) use paths::shell_arg;

/// Project paths never copied out: credentials, engine config, workflows and git.
pub(crate) use inputs::excluded as project_input_excluded;
pub fn acceptance_input_excluded(
    path: &Path,
    excludes: &[PathBuf],
    task_root: Option<&Path>,
) -> bool {
    inputs::acceptance_input_excluded(path, excludes, task_root)
}

/// Issue 440: the one nested-repository rule of every project-input walk.
pub use inputs::{nested_repository, nested_repository_skipped};
/// Whether a project-relative path is host-owned state excluded from acceptance inputs.
pub fn harness_run_state_path(path: &Path) -> bool {
    inputs::acceptance_harness_run_state(path)
}

/// Whether a path is written by freezing and cannot be check input.
pub fn freeze_owned_path(path: &Path) -> bool {
    inputs::freeze_owned(path)
}
#[path = "acceptance_scratch_faults.rs"]
pub mod faults;
#[path = "acceptance_scratch_hooks.rs"]
mod hooks;
pub use hooks::{
    AllowanceHook, CHECK_DEFERRED, CheckAllowance, CheckHook, ObserveControl, ObserveHooks,
};
#[path = "acceptance_scratch_identity.rs"]
mod identity;
pub use identity::{BuildIdentity, CheckEvidence};
#[path = "acceptance_scratch_io.rs"]
mod io;
pub use io::inventory;
#[path = "acceptance_scratch_mutation_fd.rs"]
mod mutation_fd;
#[path = "acceptance_scratch_observe.rs"]
mod observe;
#[path = "acceptance_scratch_process.rs"]
mod process;
#[path = "acceptance_scratch_seal.rs"]
mod seal;
#[path = "acceptance_scratch_teardown.rs"]
mod teardown;
pub use control::{OBSERVATION_STALLED, observation_stall};
pub use observe::AttributedTeardown;
pub use observe::observe_commands_hooked;
pub use observe::{ObservationResult, observe_commands, observe_commands_cancellable};
pub(crate) use process::drain_counted;
pub use process::{CHECK_TIMED_OUT, CheckResult, MutationInputPlan, MutationPlan};
pub use site_path::{
    SITE_PROJECT_DIRECTORY, SITE_REPOSITORY_DIRECTORY, SitePathRefusal, site_path_alone,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchPolicy {
    pub repository: PathBuf,
    pub project: PathBuf,
    pub task_root: PathBuf,
    pub scratch_parent: PathBuf,
    pub project_inputs: Vec<PathBuf>,
    #[serde(default)]
    pub project_input_excludes: Vec<PathBuf>,
    pub combined: bool,
    pub toolchain_path: String,
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    pub cargo_seed: Option<PathBuf>,
    pub timeout_secs: u64,
    pub output_bytes: usize,
    pub scratch_bytes: u64,
    /// Persistent compiled-artifact cache; `None` builds each observation cold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache: Option<PathBuf>,
}
fn invalid(message: impl Into<String>) -> WorkflowError {
    WorkflowError::SpecInvalid(message.into())
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
impl ScratchPolicy {
    pub fn validate(&self) -> WorkflowResult<()> {
        for root in [
            &self.repository,
            &self.project,
            &self.task_root,
            &self.scratch_parent,
        ]
        .into_iter()
        .chain(&self.build_cache)
        {
            if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(invalid(
                    "scratch policy roots must be absolute and normalized",
                ));
            }
        }
        if self.timeout_secs == 0 || self.output_bytes == 0 || self.scratch_bytes == 0 {
            return Err(invalid("scratch limits must be positive"));
        }
        for input in self
            .project_inputs
            .iter()
            .chain(&self.project_input_excludes)
        {
            if !relative(input) || input.components().any(|c| c.as_os_str() == ".git") {
                return Err(invalid(
                    "project input must be a normalized relative path without .git",
                ));
            }
        }
        crate::acceptance_check_environment::validate_operator_bindings(
            &self.toolchain_path,
            &self.environment,
            &self.environment_allowlist,
        )
        .map_err(invalid)?;
        Ok(())
    }
}

pub struct ScratchRoots {
    root: PathBuf,
    mutation_root: mutation_fd::SiteRoot,
    /// CARGO_TARGET_DIR: `root/target`, or the held build cache's.
    target: PathBuf,
    lease: Option<cache::Lease>,
    repository: PathBuf,
    project: PathBuf,
    live_repository: PathBuf,
    registered: bool,
    /// The repository copy's `.git` link as the live repository wrote it.
    worktree_link: Option<Vec<u8>>,
    cleaned: bool,
    cleanup_timeout_secs: u64,
    host_environment: BTreeMap<String, String>,
    siblings: siblings::Copied,
}
impl ScratchRoots {
    pub fn prepare(policy: &ScratchPolicy, commit: &str) -> WorkflowResult<Self> {
        let control = control::Control::new(
            policy.timeout_secs,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        control.run(|| Self::prepare_inner(policy, commit))
    }
    pub(super) fn prepare_inner(policy: &ScratchPolicy, commit: &str) -> WorkflowResult<Self> {
        Self::prepare_inner_with_checks(policy, commit, &[])
    }
    pub(super) fn prepare_inner_with_checks(
        policy: &ScratchPolicy,
        commit: &str,
        checks: &[(String, crate::task_set_contract::TrustedCwd)],
    ) -> WorkflowResult<Self> {
        Self::prepare_leased(policy, commit, checks, None)
    }
    /// [`Self::prepare_inner_with_checks`] under `held`, the build cache
    /// lease an earlier site of the same observation kept (`teardown`).
    fn prepare_leased(
        policy: &ScratchPolicy,
        commit: &str,
        checks: &[(String, crate::task_set_contract::TrustedCwd)],
        held: Option<cache::Lease>,
    ) -> WorkflowResult<Self> {
        policy.validate()?;
        // Issue 345: the one check-environment rule's forwarded values.
        let host_environment = crate::acceptance_check_environment::forwarded_values(
            &crate::acceptance_check_environment::host_environment(),
            &policy.environment_allowlist,
        )
        .map_err(invalid)?;
        control::check()?;
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("recorded source commit must be a full object id"));
        }
        let live_repository = std::fs::canonicalize(&policy.repository)
            .map(archon_shell::paths::plain)
            .map_err(|e| WorkflowError::io(&policy.repository, e))?;
        let project = std::fs::canonicalize(&policy.project)
            .map(archon_shell::paths::plain)
            .map_err(|e| WorkflowError::io(&policy.project, e))?;
        let tasks = std::fs::canonicalize(&policy.task_root)
            .map(archon_shell::paths::plain)
            .map_err(|e| WorkflowError::io(&policy.task_root, e))?;
        // Create the parent only after checking its existing ancestor against live roots.
        for storage in std::iter::once(&policy.scratch_parent).chain(&policy.build_cache) {
            let mut ancestor = storage.as_path();
            while !ancestor.exists() {
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| invalid("scratch parent has no existing ancestor"))?;
            }
            let canonical = std::fs::canonicalize(ancestor)
                .map(archon_shell::paths::plain)
                .map_err(|e| WorkflowError::io(ancestor, e))?;
            if [&live_repository, &project, &tasks]
                .iter()
                .any(|r| canonical.starts_with(r))
            {
                return Err(invalid("scratch storage cannot be inside live roots"));
            }
        }
        std::fs::create_dir_all(&policy.scratch_parent)
            .map_err(|e| WorkflowError::io(&policy.scratch_parent, e))?;
        let mut lease = match (held, &policy.build_cache) {
            (Some(held), _) => Some(held.renew(policy.scratch_bytes)?),
            (None, Some(dir)) => Some(cache::Lease::acquire(dir, policy.scratch_bytes)?),
            (None, None) => None,
        };
        let root = match &mut lease {
            // Keep Cargo paths stable; reclaim slots left by holders that did not tear down.
            Some(lease) => {
                // Unregistered stale slots are removed and never reused.
                for stale in lease.leftovers("scratch-") {
                    let _ = git(
                        &live_repository,
                        &["worktree", "remove", "--force"],
                        &[&stale.join(site_path::SITE_REPOSITORY_DIRECTORY)],
                    );
                    // A stale slot can be unregistered or partly damaged.
                    // Remove old slot contents when possible; cache preparation
                    // advances the generation if the current slot remains.
                    let _ = io::remove_owned_tree(&stale);
                }
                lease.prepare_slot(policy.scratch_bytes)?
            }
            None => std::fs::canonicalize(&policy.scratch_parent)
                .map(archon_shell::paths::plain)
                .map_err(|e| WorkflowError::io(&policy.scratch_parent, e))?
                .join(format!("observation-{}", uuid::Uuid::new_v4())),
        };
        cache::create_slot(&root)?;
        let mutation_root =
            mutation_fd::SiteRoot::capture(&root).map_err(|e| WorkflowError::io(&root, e))?;
        let mut roots = Self {
            repository: root.join(site_path::SITE_REPOSITORY_DIRECTORY),
            project: root.join(site_path::SITE_PROJECT_DIRECTORY),
            target: lease
                .as_ref()
                .map_or_else(|| root.join("target"), cache::Lease::target),
            lease,
            root,
            mutation_root,
            live_repository,
            registered: false,
            worktree_link: None,
            cleaned: false,
            cleanup_timeout_secs: policy.timeout_secs.max(5),
            host_environment,
            siblings: siblings::Copied::default(),
        };
        let setup = (|| {
            roots.registered = true;
            // Reclaim a fixed slot still registered by a stale holder.
            let reclaim: &[&str] = if roots.lease.is_some() { &["-f"] } else { &[] };
            let add = [
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
            ];
            git(
                &roots.live_repository,
                &[&add[..], reclaim].concat(),
                &[&roots.repository, Path::new(commit)],
            )?;
            roots.registered = true;
            let link = roots.repository.join(".git");
            roots.worktree_link =
                Some(std::fs::read(&link).map_err(|e| WorkflowError::io(&link, e))?);
            let head = git(&roots.repository, &["rev-parse", "HEAD"], &[])?;
            if head.trim() != commit {
                return Err(invalid("scratch worktree revision mismatch"));
            }
            for name in [SITE_PROJECT_DIRECTORY, "home", "tmp", "cargo-home"] {
                let path = roots.root.join(name);
                std::fs::create_dir_all(&path).map_err(|e| WorkflowError::io(&path, e))?;
            }
            std::fs::create_dir_all(&roots.target)
                .map_err(|e| WorkflowError::io(&roots.target, e))?;
            let mut remaining = policy.scratch_bytes;
            if policy.combined {
                io::copy_tree(&roots.repository, &roots.project, &mut remaining, true)?;
                std::fs::copy(roots.repository.join(".git"), roots.project.join(".git"))
                    .map_err(|e| WorkflowError::io(roots.project.join(".git"), e))?;
            }
            let project_excludes = inputs::project_excludes(policy)?;
            // Issue 440: the repository comes from its commit, never an input.
            let nested = inputs::nested_repository(&project, &roots.live_repository);
            let task_relative = tasks
                .strip_prefix(&project)
                .map_err(|_| invalid("task root must belong to declared project"))?;
            for input in &policy.project_inputs {
                if matches!(
                    input.to_str(),
                    Some(
                        "credentials"
                            | "credentials.toml"
                            | "config.toml"
                            | "config.json"
                            | ".env"
                            | ".archon/config.toml"
                    )
                ) {
                    return Err(invalid("host credential/config input cannot be exported"));
                }
                inputs::copy_project(
                    &project,
                    input,
                    &roots.project,
                    (&project_excludes, nested.as_deref()),
                    task_relative,
                    &mut remaining,
                )
                .map_err(|error| inputs::name_collision(error, policy, &project, input, commit))?;
            }
            io::copy_task_root(
                &tasks,
                &roots.project.join(task_relative),
                task_relative,
                task_relative,
                (&project_excludes, nested.as_deref()),
                &mut remaining,
            )?;
            roots.siblings = siblings::copy_external(
                &project,
                &policy.repository,
                &roots.project,
                &roots.repository,
                checks,
                &mut remaining,
            )?;
            io::readonly(&roots.project.join(task_relative))?;
            if let Some(seed) = &policy.cargo_seed {
                // Copy only standard content-addressed cache areas, not credentials/config.
                for name in ["registry", "git"] {
                    let source = seed.join(name);
                    if source.exists() {
                        io::copy_cache(
                            &source,
                            &roots.root.join("cargo-home").join(name),
                            &mut remaining,
                        )?;
                    }
                }
            }
            if let Some(lease) = roots.lease.as_mut() {
                let listed = git(
                    &roots.repository,
                    &["ls-tree", "-r", "-z", "--name-only", "HEAD"],
                    &[],
                )?;
                let names: Vec<&str> = (listed.split('\0'))
                    .filter(|name| relative(Path::new(name)))
                    .collect();
                let mut sources = vec![(
                    site_path::SITE_REPOSITORY_DIRECTORY,
                    roots.repository.as_path(),
                )];
                if policy.combined {
                    sources.push((site_path::SITE_PROJECT_DIRECTORY, roots.project.as_path()));
                }
                lease.stabilize(&sources, &names)?;
            }
            for cwd in [&roots.project, &roots.repository] {
                let target = cwd.join("target");
                if target.symlink_metadata().is_ok() {
                    return Err(invalid(
                        "source/project target path conflicts with scratch build target",
                    ));
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(roots.target(), &target)
                    .map_err(|e| WorkflowError::io(&target, e))?;
                #[cfg(not(unix))]
                return Err(invalid(
                    "native scratch target links require a supported Unix host",
                ));
            }
            let snapshot = roots.root.join("project-baseline");
            inputs::snapshot_project(&roots.project, &snapshot, &mut remaining)?;
            Ok(())
        })();
        if let Err(error) = setup {
            if let Err(cleanup) = roots.cleanup() {
                return Err(invalid(format!(
                    "scratch setup failed: {error}; cleanup failed: {cleanup}"
                )));
            }
            return Err(error);
        }
        Ok(roots)
    }
    pub(super) fn reset_project(&self) -> WorkflowResult<()> {
        inputs::reset_project(self)
    }
    pub fn source_inventory(&self) -> WorkflowResult<BTreeMap<String, String>> {
        let mut result = BTreeMap::new();
        let output = git(
            &self.repository,
            &["ls-tree", "-r", "--name-only", "HEAD"],
            &[],
        )?;
        for name in output.lines() {
            let path = Path::new(name);
            if !relative(path) {
                return Err(invalid("recorded source contains an unsafe path"));
            }
            for (label, root) in [
                (site_path::SITE_REPOSITORY_DIRECTORY, &self.repository),
                (site_path::SITE_PROJECT_DIRECTORY, &self.project),
            ] {
                let file = root.join(path);
                if label == site_path::SITE_PROJECT_DIRECTORY && !file.exists() {
                    continue;
                }
                for (suffix, digest) in inventory(&file)? {
                    result.insert(format!("{label}/{name}/{suffix}"), digest);
                }
            }
        }
        Ok(result)
    }
    /// A check's variables here but the forwarded values (Issue 345's one
    /// rule, `acceptance_check_environment`): the policy's, and this
    /// scratch's own HOME, TMPDIR, CARGO_HOME and build directory.
    pub fn environment(&self, policy: &ScratchPolicy) -> BTreeMap<String, String> {
        use crate::acceptance_check_environment as rule;
        let (home, tmp, cargo) = (
            self.root.join("home"),
            self.root.join("tmp"),
            self.root.join("cargo-home"),
        );
        // Windows: the profile variables point at this scratch's home too.
        let profile = rule::profile_bindings(&home);
        let mut site: Vec<(&str, &Path)> = vec![("HOME", &home), ("TMPDIR", &tmp)];
        site.extend([
            ("CARGO_HOME", cargo.as_path()),
            ("CARGO_TARGET_DIR", &self.target),
        ]);
        site.extend(profile.iter().map(|(name, path)| (*name, path.as_path())));
        let policy = rule::CheckPolicy::configured(policy);
        rule::site_variables(&rule::host_environment(), &policy, &site)
    }
    pub(super) fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        self.redact_output(bytes, false)
    }
    pub(super) fn redact_output(&self, bytes: &[u8], truncated: bool) -> Vec<u8> {
        io::redact(&self.host_environment, bytes, truncated)
    }
}
