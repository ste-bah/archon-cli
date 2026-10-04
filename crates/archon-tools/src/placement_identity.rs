//! Which directory an agent was placed in, so a holder can tell that the
//! directory it finds at that path later is still that one (#241).
//!
//! A path alone does not say it. A directory can be removed and created
//! again; a linked worktree can be moved away and its old path given a
//! `.git` pointer to the same administrative directory; a worktree can be
//! removed and registered again under the same name and branch. None of
//! these is the place the agent was confined to. So the identity holds the
//! directory's generation (its file identity: device and inode, or on Windows
//! volume serial and file index; and its creation time where recorded), and,
//! inside a git repository, the working tree the repository itself registers
//! for it and the generation of its git directory. A directory whose file
//! identity cannot be read has no placement: a path and a writable creation
//! time alone cannot tell a replacement apart.
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
    /// Device and inode, or volume serial and file index.
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

    /// `Ok` when `path` is still this placement. The repository is compared
    /// only when there was one: an agent may legitimately create one inside
    /// the directory it was given.
    pub fn check(&self, path: &Path) -> Result<(), String> {
        let now = Self::of(path)?;
        if now.dir == self.dir && (self.git.is_none() || now.git == self.git) {
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
        let node = file_identity(&path, &meta);
        Self::from_parts(path, node, meta.created().ok())
    }

    fn from_parts(
        path: PathBuf,
        node: Option<(u64, u64)>,
        born: Option<std::time::SystemTime>,
    ) -> Result<Self, String> {
        let node = node.ok_or_else(|| {
            format!(
                "the file identity of {} cannot be read, so a replacement could not be told apart",
                path.display()
            )
        })?;
        Ok(Self { path, node, born })
    }
}

#[cfg(unix)]
fn file_identity(_path: &Path, meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(windows)]
fn file_identity(path: &Path, _meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    windows_identity::of(path)
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_path: &Path, _meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
#[path = "placement_identity_windows.rs"]
mod windows_identity;

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
