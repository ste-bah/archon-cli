//! Native observation roots. Construction and audit, not a hostile-code sandbox.
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[path = "acceptance_scratch_control.rs"]
mod control;
#[path = "acceptance_scratch_inputs.rs"]
mod inputs;
use control::git;
#[path = "acceptance_scratch_identity.rs"]
mod identity;
pub use identity::{BuildIdentity, CheckEvidence};
#[path = "acceptance_scratch_io.rs"]
mod io;
pub use io::inventory;
#[path = "acceptance_scratch_observe.rs"]
mod observe;
#[path = "acceptance_scratch_process.rs"]
mod process;
pub use observe::{ObservationResult, observe_commands, observe_commands_cancellable};
pub use process::CheckResult;

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
    pub cargo_seed: Option<PathBuf>,
    pub timeout_secs: u64,
    pub output_bytes: usize,
    pub scratch_bytes: u64,
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
        ] {
            if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(invalid(
                    "scratch policy roots must be absolute and normalized",
                ));
            }
        }
        if self.timeout_secs == 0 || self.output_bytes == 0 || self.scratch_bytes == 0 {
            return Err(invalid("scratch limits must be positive"));
        }
        if self
            .toolchain_path
            .split(':')
            .any(|p| p.is_empty() || !Path::new(p).is_absolute())
        {
            return Err(invalid("toolchain PATH must contain absolute directories"));
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
        for (key, value) in &self.environment {
            // Only runtime-neutral, nonsecret knobs. Expand by reviewed host policy,
            // not by accepting arbitrary names with a denylist of known secrets.
            if !matches!(
                key.as_str(),
                "LANG" | "LC_ALL" | "TZ" | "RUSTUP_HOME" | "RUSTUP_TOOLCHAIN"
            ) || value.contains('\0')
            {
                return Err(invalid(format!(
                    "environment key '{key}' is not a permitted nonsecret scratch binding"
                )));
            }
        }
        Ok(())
    }
}

pub struct ScratchRoots {
    root: PathBuf,
    repository: PathBuf,
    project: PathBuf,
    live_repository: PathBuf,
    registered: bool,
    cleaned: bool,
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
        policy.validate()?;
        control::check()?;
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("recorded source commit must be a full object id"));
        }
        let live_repository = std::fs::canonicalize(&policy.repository)
            .map_err(|e| WorkflowError::io(&policy.repository, e))?;
        let project = std::fs::canonicalize(&policy.project)
            .map_err(|e| WorkflowError::io(&policy.project, e))?;
        let tasks = std::fs::canonicalize(&policy.task_root)
            .map_err(|e| WorkflowError::io(&policy.task_root, e))?;
        // Create the parent only after checking its existing ancestor against live roots.
        let mut ancestor = policy.scratch_parent.as_path();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| invalid("scratch parent has no existing ancestor"))?;
        }
        let canonical = ancestor
            .canonicalize()
            .map_err(|e| WorkflowError::io(ancestor, e))?;
        if [&live_repository, &project, &tasks]
            .iter()
            .any(|r| canonical.starts_with(r))
        {
            return Err(invalid("scratch storage cannot be inside live roots"));
        }
        std::fs::create_dir_all(&policy.scratch_parent)
            .map_err(|e| WorkflowError::io(&policy.scratch_parent, e))?;
        let root = policy
            .scratch_parent
            .canonicalize()
            .map_err(|e| WorkflowError::io(&policy.scratch_parent, e))?
            .join(format!("observation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).map_err(|e| WorkflowError::io(&root, e))?;
        let mut roots = Self {
            repository: root.join("repo"),
            project: root.join("project"),
            root,
            live_repository,
            registered: false,
            cleaned: false,
        };
        let setup = (|| {
            roots.registered = true;
            git(
                &roots.live_repository,
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "worktree",
                    "add",
                    "--detach",
                ],
                &[&roots.repository, Path::new(commit)],
            )?;
            roots.registered = true;
            let head = git(&roots.repository, &["rev-parse", "HEAD"], &[])?;
            if head.trim() != commit {
                return Err(invalid("scratch worktree revision mismatch"));
            }
            for name in ["project", "target", "home", "tmp", "cargo-home"] {
                let path = roots.root.join(name);
                std::fs::create_dir_all(&path).map_err(|e| WorkflowError::io(&path, e))?;
            }
            let mut remaining = policy.scratch_bytes;
            if policy.combined {
                io::copy_tree(&roots.repository, &roots.project, &mut remaining, true)?;
                // Linked worktree metadata points to this same recorded source commit.
                std::fs::copy(roots.repository.join(".git"), roots.project.join(".git"))
                    .map_err(|e| WorkflowError::io(roots.project.join(".git"), e))?;
            }
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
                    &policy.project_input_excludes,
                    &mut remaining,
                )?;
            }
            let task_relative = tasks
                .strip_prefix(&project)
                .map_err(|_| invalid("task root must belong to declared project"))?;
            io::copy_tree(
                &tasks,
                &roots.project.join(task_relative),
                &mut remaining,
                false,
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
            for (label, root) in [("repo", &self.repository), ("project", &self.project)] {
                let file = root.join(path);
                if label == "project" && !file.exists() {
                    continue;
                }
                for (suffix, digest) in inventory(&file)? {
                    result.insert(format!("{label}/{name}/{suffix}"), digest);
                }
            }
        }
        Ok(result)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn project(&self) -> &Path {
        &self.project
    }
    pub fn repository(&self) -> &Path {
        &self.repository
    }
    pub fn target(&self) -> PathBuf {
        self.root.join("target")
    }
    pub fn environment(&self, policy: &ScratchPolicy) -> BTreeMap<String, String> {
        let mut env = policy.environment.clone();
        env.insert("PATH".into(), policy.toolchain_path.clone());
        for (key, path) in [
            ("HOME", "home"),
            ("TMPDIR", "tmp"),
            ("CARGO_HOME", "cargo-home"),
            ("CARGO_TARGET_DIR", "target"),
        ] {
            env.insert(
                key.into(),
                self.root.join(path).to_string_lossy().into_owned(),
            );
        }
        env
    }
    pub fn cleanup(&mut self) -> WorkflowResult<()> {
        let control = control::Control::new(
            5,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        control.run(|| self.cleanup_inner())
    }
    fn cleanup_inner(&mut self) -> WorkflowResult<()> {
        if self.cleaned {
            return Ok(());
        }
        if self.registered {
            git(
                &self.live_repository,
                &["worktree", "remove", "--force"],
                &[&self.repository],
            )?;
            let listed = git(
                &self.live_repository,
                &["worktree", "list", "--porcelain"],
                &[],
            )?;
            let expected = format!("worktree {}", self.repository.display());
            if listed.lines().any(|line| line == expected) {
                return Err(invalid(
                    "owned scratch worktree remains registered after removal",
                ));
            }
            self.registered = false;
        }
        io::remove_owned_tree(&self.root)?;
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for ScratchRoots {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}
