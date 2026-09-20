//! The repository a task set was decomposed against, recorded beside it
//! (Issue-55).
//!
//! # Why a record
//!
//! A decomposition's authors were told to read "repository source under the
//! project root", and the project root was the working directory. When the
//! project directory is not the code repository, every author globs an empty
//! tree and writes tasks asserting that files which exist "do not exist".
//! Live: 105 decomposition runs read the repository between zero and three
//! times each. The launcher now takes the repository explicitly, verifies it
//! is a git checkout, and writes this record into the task-set directory so
//! that every later reader — the body gate, the set gate, the implementation
//! run — checks claims against the same tree the authors were pointed at.
//!
//! # What is recorded
//!
//! The canonical absolute repository root, the commit the checkout was at
//! when the decomposition launched (`base_commit`, or the literal `unborn`
//! for a freshly initialised repository with no commit yet), and the id of
//! the decomposition run that wrote it. The record is written once and is
//! never rewritten by a later launch on the same task root: a frozen chain
//! was authored against this base, and a relaunch verifies the same
//! repository path and reports — never hides — a base that has moved.
//!
//! Everything here is repository-agnostic: no path, PRD or task id appears
//! in this module, and the git it speaks is the plumbing every checkout
//! answers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{WorkflowError, WorkflowResult};

/// File name of the record, beside `acceptance-contract.lock` and
/// `task-skeleton.lock` in the task-set directory.
pub const REPOSITORY_LOCK_FILE: &str = "repository.lock";
/// The `base_commit` of a repository whose `HEAD` names no commit yet.
pub const UNBORN_BASE_COMMIT: &str = "unborn";
pub const REPOSITORY_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRecordV1 {
    pub schema_version: u32,
    /// Canonical absolute path of the checkout the authors were grounded in.
    pub repository_root: String,
    /// `git rev-parse HEAD` of that checkout at launch, or [`UNBORN_BASE_COMMIT`].
    pub base_commit: String,
    /// The decomposition run that wrote this record.
    pub decomposition_run_id: String,
    pub recorded_at: String,
}

pub fn repository_record_path(task_root: &Path) -> PathBuf {
    task_root.join(REPOSITORY_LOCK_FILE)
}

/// The record under `task_root`, `None` when the task set predates it. A
/// record that exists but cannot be read or parsed is an error: a task set
/// that says which repository it was authored against must be believed or
/// repaired, never silently treated as a legacy set.
pub fn read_repository_record(task_root: &Path) -> WorkflowResult<Option<RepositoryRecordV1>> {
    let path = repository_record_path(task_root);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::io(&path, source))?;
    let record: RepositoryRecordV1 = serde_json::from_slice(&bytes).map_err(|error| {
        WorkflowError::ArtifactInvalid(format!(
            "repository record {} is malformed: {error}; restore it or remove the task set",
            path.display()
        ))
    })?;
    if record.schema_version != REPOSITORY_RECORD_SCHEMA_VERSION {
        return Err(WorkflowError::ArtifactInvalid(format!(
            "repository record {} has schema_version {}, this binary reads {}",
            path.display(),
            record.schema_version,
            REPOSITORY_RECORD_SCHEMA_VERSION
        )));
    }
    Ok(Some(record))
}

/// Write the record atomically (temp file then rename) so a reader never sees
/// a half-written document.
pub fn write_repository_record(
    task_root: &Path,
    record: &RepositoryRecordV1,
) -> WorkflowResult<()> {
    std::fs::create_dir_all(task_root).map_err(|source| WorkflowError::io(task_root, source))?;
    let path = repository_record_path(task_root);
    let temp = task_root.join(format!("{REPOSITORY_LOCK_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(record)?;
    std::fs::write(&temp, bytes).map_err(|source| WorkflowError::io(&temp, source))?;
    std::fs::rename(&temp, &path).map_err(|source| WorkflowError::io(&path, source))
}

/// `git rev-parse --show-toplevel` from `path`, when `path` is inside a
/// git working tree.
pub fn git_toplevel(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Is `path` a git checkout: `.git` exists beside it as a directory or a
/// file (a worktree or submodule), or git itself reports a top level.
pub fn is_git_checkout(path: &Path) -> bool {
    path.is_dir() && (path.join(".git").exists() || git_toplevel(path).is_some())
}

/// The commit `HEAD` names in the checkout at `repository_root`, or
/// [`UNBORN_BASE_COMMIT`] when the repository has no commit yet. An error
/// means the path is not a repository git can read.
pub fn git_head(repository_root: &Path) -> WorkflowResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .map_err(|source| WorkflowError::io(repository_root, source))?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Ok(sha);
        }
    }
    // `--verify -q` exits 1 for an unborn HEAD and 128 outside a repository.
    if output.status.code() == Some(1) && is_git_checkout(repository_root) {
        return Ok(UNBORN_BASE_COMMIT.to_string());
    }
    Err(WorkflowError::SpecInvalid(format!(
        "{} is not a git checkout this binary can read HEAD from: {}",
        repository_root.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// What is known about one repository-relative path, read from both the
/// recorded base commit and the checkout on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathTruth {
    /// Present in the tree at the recorded base commit. Always `false` for an
    /// unborn base: an empty history holds nothing.
    pub at_base: bool,
    /// Present in the working tree of the recorded checkout right now.
    pub in_checkout: bool,
}

impl PathTruth {
    /// Certainly present: both the base commit and the checkout hold it, so
    /// a text saying it does not exist is wrong whichever one its author read.
    pub fn certainly_exists(self) -> bool {
        self.at_base && self.in_checkout
    }

    /// Certainly absent: neither the base commit nor the checkout holds it.
    pub fn certainly_absent(self) -> bool {
        !self.at_base && !self.in_checkout
    }
}

/// The tree at a recorded base commit, listed once, plus the checkout it was
/// recorded from. Built by every gate that checks a text's claims about
/// repository paths against the repository the decomposition was grounded in.
#[derive(Debug, Clone)]
pub struct RepositoryTree {
    root: PathBuf,
    base_commit: String,
    /// Every file and directory path at the base commit, relative to `root`,
    /// with forward slashes and no trailing slash.
    at_base: BTreeSet<String>,
}

impl RepositoryTree {
    /// List the recorded base commit's tree under `record.repository_root`.
    /// An unborn base lists nothing; a base git cannot list is an error.
    pub fn load(record: &RepositoryRecordV1) -> WorkflowResult<Self> {
        let root = PathBuf::from(&record.repository_root);
        if !root.is_dir() {
            return Err(WorkflowError::SpecInvalid(format!(
                "recorded repository root {} is not a directory; the task set was decomposed against a checkout that is no longer there",
                root.display()
            )));
        }
        let mut at_base = BTreeSet::new();
        if record.base_commit != UNBORN_BASE_COMMIT {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "ls-tree",
                    "-r",
                    "-t",
                    "-z",
                    "--name-only",
                    &record.base_commit,
                ])
                .output()
                .map_err(|source| WorkflowError::io(&root, source))?;
            if !output.status.success() {
                return Err(WorkflowError::SpecInvalid(format!(
                    "recorded base commit {} cannot be listed in {}: {}",
                    record.base_commit,
                    root.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            for entry in output.stdout.split(|byte| *byte == 0) {
                let text = String::from_utf8_lossy(entry);
                let text = text.trim_end_matches('/');
                if text.is_empty() || text == "." {
                    continue;
                }
                at_base.insert(text.to_string());
            }
        }
        Ok(Self {
            root,
            base_commit: record.base_commit.clone(),
            at_base,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    /// Every path at the base commit, for callers that match many paths at
    /// once (owner coverage).
    pub fn paths_at_base(&self) -> &BTreeSet<String> {
        &self.at_base
    }

    /// Is `relative` a file or directory at the base commit.
    pub fn exists_at_base(&self, relative: &str) -> bool {
        self.at_base.contains(normalize_relative(relative).as_str())
    }

    /// Is `relative` a directory at the base commit: it is listed as a tree,
    /// or some listed path sits under it.
    pub fn is_dir_at_base(&self, relative: &str) -> bool {
        let normalized = normalize_relative(relative);
        let prefix = format!("{normalized}/");
        self.at_base
            .range(prefix.clone()..)
            .next()
            .is_some_and(|path| path.starts_with(&prefix))
    }

    pub fn truth(&self, relative: &str) -> PathTruth {
        let normalized = normalize_relative(relative);
        PathTruth {
            at_base: self.at_base.contains(&normalized),
            in_checkout: self.root.join(&normalized).exists(),
        }
    }

    /// Strip the repository root from an absolute path, or normalise a
    /// relative one. `None` when the absolute path lies outside the root.
    pub fn relative_to_root(&self, path: &str) -> Option<String> {
        #[cfg(windows)]
        let path = path.replace('/', "\\");
        let candidate = Path::new(&path);
        if candidate.is_absolute() || candidate.has_root() {
            let stripped = candidate.strip_prefix(&self.root).ok()?;
            return Some(normalize_relative(&stripped.to_string_lossy()));
        }
        Some(normalize_relative(&path))
    }
}

/// `./a//b/` → `a/b`; backslashes become forward slashes.
pub fn normalize_relative(path: &str) -> String {
    let forward = path.replace('\\', "/");
    forward
        .split('/')
        .filter(|part| !matches!(*part, "" | "."))
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
#[path = "repository_record_tests.rs"]
mod tests;
