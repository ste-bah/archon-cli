//! Where a spawned agent works, decided ONCE at spawn (Issue-213 C3).
//!
//! A write agent the host placed in its own worktree edited five files in the
//! canonical checkout. Its record named the worktree as its repository root,
//! and both were true at once: the agent RAN in the worktree while its file
//! tools could still WRITE the checkout, because the checkout came along as an
//! inherited directory and nothing said that being isolated from a tree means
//! not writing it.
//!
//! So placement is one value with two shapes, never both:
//!
//! - [`Placement::WorkingDir`]: the agent works in a directory it shares with
//!   its parent's world, exactly as before;
//! - [`Placement::Isolated`]: the agent works in an isolated checkout of a
//!   repository, and every OTHER checkout of that repository — the one it was
//!   taken from, its siblings, and any made after it was spawned — is sealed:
//!   readable, never written by its file tools.
//!
//! A seal names the repository (its git common directory), not a list of
//! checkouts: each write is judged against the checkout that owns the path
//! ([`owning_checkout`]), found by walking up to the nearest `.git`. That is a
//! few `stat`s per write however many worktrees the repository has, and it
//! covers a worktree created after the spawn.
//!
//! Only in a workflow context (a run store or a workflow read guard in scope):
//! an interactive subagent keeps exactly the access it had, as the workflow
//! write-confinement setting already promises. No tool, language or project is
//! named; every fact is read from the filesystem.

use std::path::{Path, PathBuf};

use crate::tool::ToolContext;

/// Where one spawned agent works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Works in this directory, in its parent's world.
    WorkingDir(PathBuf),
    /// Works in `workspace`, a checkout of the repository whose common git
    /// directory is `repository`; the repository's other checkouts are sealed.
    Isolated {
        workspace: PathBuf,
        repository: PathBuf,
    },
}

/// Whether `ctx` runs inside a workflow, the only context seals apply in.
pub fn in_workflow(ctx: &ToolContext) -> bool {
    ctx.run_store.is_some() || ctx.workflow_read_guard.is_some()
}

impl Placement {
    /// The directory a child of `parent` starts from, before any worktree is
    /// made for it.
    ///
    /// Outside a workflow this is what it always was: the requested directory,
    /// else `fallback` (the executor's own). Inside one, a child that names no
    /// directory works where its parent does, and a child that names a
    /// directory its parent may not write is placed in its parent's workspace
    /// instead: a seal is inherited, never escaped by spawning.
    pub fn child_dir(parent: &ToolContext, requested: Option<&Path>, fallback: &Path) -> PathBuf {
        let parent_dir =
            (!parent.working_dir.as_os_str().is_empty()).then_some(parent.working_dir.as_path());
        if !in_workflow(parent) {
            return requested.unwrap_or(fallback).to_path_buf();
        }
        let dir = requested.or(parent_dir).unwrap_or(fallback);
        match parent_dir {
            Some(own) if sealed_for(dir, own, &parent.sealed_repositories) => own.to_path_buf(),
            _ => dir.to_path_buf(),
        }
    }

    /// Decide placement for a child of `parent` starting from `dir`, given the
    /// worktree the spawn made for it, if any.
    pub fn resolve(parent: &ToolContext, dir: &Path, created_worktree: Option<&Path>) -> Self {
        let workspace = created_worktree.unwrap_or(dir);
        if !in_workflow(parent) {
            return Self::WorkingDir(workspace.to_path_buf());
        }
        let isolated = created_worktree.is_some() || {
            let parent_checkout = owning_checkout(&parent.working_dir).map(|c| c.checkout);
            owning_checkout(dir).is_some_and(|own| {
                own.linked && parent_checkout.is_none_or(|p| !same_dir(&p, &own.checkout))
            })
        };
        match owning_checkout(workspace).filter(|_| isolated) {
            Some(own) => Self::Isolated {
                workspace: workspace.to_path_buf(),
                repository: own.repository,
            },
            None => Self::WorkingDir(workspace.to_path_buf()),
        }
    }

    /// The directory the agent runs in.
    pub fn working_dir(&self) -> &Path {
        match self {
            Self::WorkingDir(dir) => dir,
            Self::Isolated { workspace, .. } => workspace,
        }
    }

    /// The repository this placement seals, if any.
    pub fn sealed_repository(&self) -> Option<&Path> {
        match self {
            Self::WorkingDir(_) => None,
            Self::Isolated { repository, .. } => Some(repository),
        }
    }
}

/// The checkout a path belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwningCheckout {
    /// The checkout's root: the directory holding its `.git`.
    pub checkout: PathBuf,
    /// The repository's common git directory, canonicalised.
    pub repository: PathBuf,
    /// Whether the checkout is a linked worktree (its `.git` is a file).
    pub linked: bool,
}

/// The checkout that owns `path`: the nearest ancestor (or `path` itself)
/// holding a `.git`. `None` outside any checkout, and for a relative or empty
/// path, which would be judged against whatever directory this process is in.
/// `path` need not exist.
pub fn owning_checkout(path: &Path) -> Option<OwningCheckout> {
    if !path.is_absolute() {
        return None;
    }
    for dir in path.ancestors() {
        let dot_git = dir.join(".git");
        let Ok(meta) = std::fs::metadata(&dot_git) else {
            continue;
        };
        let (repository, linked) = if meta.is_dir() {
            (dot_git, false)
        } else {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = dir.join(text.trim().strip_prefix("gitdir:")?.trim());
            // A linked worktree's git directory names the shared one; a
            // submodule's names none and is its own repository.
            match std::fs::read_to_string(gitdir.join("commondir")) {
                Ok(relative) => (gitdir.join(relative.trim()), true),
                Err(_) => (gitdir, false),
            }
        };
        return Some(OwningCheckout {
            checkout: dir.to_path_buf(),
            repository: std::fs::canonicalize(&repository).unwrap_or(repository),
            linked,
        });
    }
    None
}

/// Whether `path` lies in a checkout of a sealed repository other than the one
/// that owns `own_dir`.
pub fn sealed_for(path: &Path, own_dir: &Path, sealed: &[PathBuf]) -> bool {
    sealed_checkout(path, own_dir, sealed).is_some()
}

/// The sealed checkout `path` lies in, if [`sealed_for`].
pub fn sealed_checkout(path: &Path, own_dir: &Path, sealed: &[PathBuf]) -> Option<PathBuf> {
    if sealed.is_empty() {
        return None;
    }
    let owner = owning_checkout(path)?;
    if !sealed.iter().any(|repo| same_dir(repo, &owner.repository)) {
        return None;
    }
    let own = owning_checkout(own_dir).map(|c| c.checkout);
    if own.is_some_and(|own| same_dir(&own, &owner.checkout)) {
        return None;
    }
    Some(owner.checkout)
}

/// Whether two spellings name the same directory (`/var` is `/private/var`).
fn same_dir(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || real(a) == real(b)
}

#[cfg(test)]
#[path = "spawn_placement_tests.rs"]
mod tests;
