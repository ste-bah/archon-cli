//! Which directory an agent was placed in, so a holder can tell that the
//! directory it finds at that path later is still that one (#241).
//!
//! A path alone does not say it. A directory can be removed and created
//! again; a linked worktree can be moved away and its old path given a
//! `.git` pointer to the same administrative directory; a worktree can be
//! removed and registered again under the same name and branch. None of
//! these is the place the agent was confined to. So the identity holds the
//! directory's generation (its inode and creation time), and, inside a git
//! repository, the working tree the repository itself registers for it and
//! the generation of its git directory.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementIdentity {
    dir: Stamp,
    git: Option<GitPlacement>,
}

/// One generation of one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    /// Device and inode where the platform has them; zero elsewhere.
    node: (u64, u64),
    /// Creation time where the file system records one.
    born: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitPlacement {
    /// The working tree the repository registers: for a linked worktree,
    /// read from its administrative directory, not from the pointer.
    workdir: PathBuf,
    git_dir: Stamp,
    common_dir: PathBuf,
    /// Recorded for a linked worktree only. An agent's own commits move HEAD
    /// along the branch, which is the same placement.
    branch: Option<String>,
}

impl PlacementIdentity {
    /// The identity of the directory at `path`, or why it has none.
    pub fn of(path: &Path) -> Result<Self, String> {
        let dir = Stamp::of(path)?;
        let git = match git2::Repository::discover(&dir.path) {
            Ok(repo) => Some(GitPlacement::of(&repo, &dir.path)?),
            Err(error) if error.code() == git2::ErrorCode::NotFound => None,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        Ok(Self { dir, git })
    }

    /// `Ok` when `path` is still this placement.
    pub fn check(&self, path: &Path) -> Result<(), String> {
        let now = Self::of(path)?;
        if now == *self {
            return Ok(());
        }
        Err(format!(
            "{} is no longer the directory it was ({self:?} then, {now:?} now)",
            path.display()
        ))
    }
}

impl Stamp {
    fn of(path: &Path) -> Result<Self, String> {
        let path =
            std::fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let meta =
            std::fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", path.display()));
        }
        #[cfg(unix)]
        let node = {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino())
        };
        #[cfg(not(unix))]
        let node = (0, 0);
        Ok(Self {
            node,
            born: meta.created().ok(),
            path,
        })
    }
}

impl GitPlacement {
    fn of(repo: &git2::Repository, dir: &Path) -> Result<Self, String> {
        let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let workdir = repo
            .workdir()
            .map(real)
            .ok_or_else(|| format!("{} is inside a bare repository", dir.display()))?;
        if !dir.starts_with(&workdir) {
            return Err(format!(
                "{} is not inside the working tree its repository registers ({})",
                dir.display(),
                workdir.display()
            ));
        }
        let linked = repo.is_worktree();
        Ok(Self {
            workdir,
            git_dir: Stamp::of(repo.path())?,
            common_dir: real(repo.commondir()),
            branch: linked
                .then(|| repo.head().ok()?.name().map(str::to_string))
                .flatten(),
        })
    }
}

#[cfg(test)]
#[path = "placement_identity_tests.rs"]
mod tests;
