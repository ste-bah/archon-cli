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
//! - [`Placement::Isolated`]: the agent works in an isolated workspace, and
//!   every OTHER checkout of the same repository — the one it was taken from,
//!   and its siblings — is sealed: readable, never written by its file tools.
//!
//! A workspace counts as isolated when the spawn made a worktree for it, or
//! when the caller placed it in a linked git worktree other than its own
//! directory (a host's per-branch worktree). Both facts are read from the
//! filesystem at spawn; no tool, language or project is named. The seal is
//! carried in `ToolContext::sealed_roots`, and a child inherits its parent's
//! seals on top of its own, so a bound only ever narrows down the tree.

use std::path::{Path, PathBuf};

/// Where one spawned agent works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Works in this directory, in its parent's world.
    WorkingDir(PathBuf),
    /// Works in `workspace`; the repository's other checkouts are `sealed`.
    Isolated {
        workspace: PathBuf,
        sealed: Vec<PathBuf>,
    },
}

impl Placement {
    /// Decide placement from the parent's directory, the directory the
    /// request named, and the worktree the spawn made for it, if any.
    pub fn resolve(
        parent_dir: &Path,
        requested_cwd: Option<&Path>,
        created_worktree: Option<&Path>,
    ) -> Self {
        let dir = requested_cwd.unwrap_or(parent_dir);
        if let Some(workspace) = created_worktree {
            let mut sealed = other_checkouts(workspace);
            // The source is sealed even when git cannot list it (a source
            // that is not a checkout at all still must not be written).
            if !same_dir(dir, workspace) && !sealed.iter().any(|s| same_dir(s, dir)) {
                sealed.push(dir.to_path_buf());
            }
            return Self::Isolated {
                workspace: workspace.to_path_buf(),
                sealed,
            };
        }
        if !same_dir(dir, parent_dir) {
            let sealed = other_checkouts(dir);
            if !sealed.is_empty() {
                return Self::Isolated {
                    workspace: dir.to_path_buf(),
                    sealed,
                };
            }
        }
        Self::WorkingDir(dir.to_path_buf())
    }

    /// The directory the agent runs in.
    pub fn working_dir(&self) -> &Path {
        match self {
            Self::WorkingDir(dir) => dir,
            Self::Isolated { workspace, .. } => workspace,
        }
    }

    /// The checkouts the agent may read and never write.
    pub fn sealed_roots(&self) -> &[PathBuf] {
        match self {
            Self::WorkingDir(_) => &[],
            Self::Isolated { sealed, .. } => sealed,
        }
    }
}

/// Every checkout of `dir`'s repository except `dir` itself, when `dir` is a
/// linked worktree; empty for an ordinary checkout or a plain directory, which
/// have no isolation to speak of.
fn other_checkouts(dir: &Path) -> Vec<PathBuf> {
    let Some(common) = linked_common_dir(dir) else {
        return Vec::new();
    };
    let mut checkouts = Vec::new();
    // The main checkout holds the common directory as its `.git`.
    if common.file_name().is_some_and(|name| name == ".git")
        && let Some(main) = common.parent()
    {
        checkouts.push(main.to_path_buf());
    }
    // Every linked worktree records where its `.git` file is.
    if let Ok(entries) = std::fs::read_dir(common.join("worktrees")) {
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("gitdir")) else {
                continue;
            };
            if let Some(checkout) = Path::new(text.trim()).parent() {
                checkouts.push(checkout.to_path_buf());
            }
        }
    }
    checkouts.retain(|checkout| !same_dir(checkout, dir));
    checkouts.sort();
    checkouts.dedup();
    checkouts
}

/// The repository's common git directory, when `dir` is a linked worktree
/// (its `.git` is a file naming a git directory with a `commondir`).
fn linked_common_dir(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(".git")).ok()?;
    let gitdir = dir.join(text.trim().strip_prefix("gitdir:")?.trim());
    let relative = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = gitdir.join(relative.trim());
    Some(std::fs::canonicalize(&common).unwrap_or(common))
}

/// Whether two spellings name the same directory (`/var` is `/private/var`).
fn same_dir(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || real(a) == real(b)
}

#[cfg(test)]
#[path = "spawn_placement_tests.rs"]
mod tests;
