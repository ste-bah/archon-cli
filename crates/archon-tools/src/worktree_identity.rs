//! Which git worktree a directory is, so a holder can tell that the
//! directory it finds later is still that worktree (#241).
//!
//! A path alone does not say it. A worktree removed at completion can be
//! replaced by a plain directory at the same path, and a worktree whose
//! registration in its repository was removed is a directory with a stale
//! `.git` pointer. Neither is the place an agent was confined to.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeIdentity {
    /// The worktree's own git directory, under the repository's `worktrees/`.
    git_dir: PathBuf,
    /// The repository the worktree belongs to.
    common_dir: PathBuf,
    /// The branch the worktree has checked out.
    branch: Option<String>,
}

impl WorktreeIdentity {
    /// The identity of the worktree at `path`, or why it is not one.
    pub fn of(path: &Path) -> Result<Self, String> {
        let repo = git2::Repository::open(path)
            .map_err(|error| format!("{} is not a git worktree: {error}", path.display()))?;
        if !repo.is_worktree() {
            return Err(format!("{} is not a linked git worktree", path.display()));
        }
        let real = |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        Ok(Self {
            git_dir: real(repo.path()),
            common_dir: real(repo.commondir()),
            branch: repo
                .head()
                .ok()
                .and_then(|head| head.name().map(str::to_string)),
        })
    }

    /// `Ok` when `path` is still this worktree.
    pub fn check(&self, path: &Path) -> Result<(), String> {
        let now = Self::of(path)?;
        if now == *self {
            return Ok(());
        }
        Err(format!(
            "{} is no longer the worktree it was ({self:?} then, {now:?} now)",
            path.display()
        ))
    }
}

#[cfg(test)]
#[path = "worktree_identity_tests.rs"]
mod tests;
