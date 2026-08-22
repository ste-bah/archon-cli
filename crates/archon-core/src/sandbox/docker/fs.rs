//! The filesystem of a docker sandbox's workspace (#201 Phase 2).
//!
//! The workspace is bind-mounted (`type=bind,src={working_dir},dst=/workspace`),
//! so the container and the host hold the *same bytes*. Reading them locally is
//! therefore correct, and nothing is gained by routing a `cat` through
//! `docker run`.
//!
//! What is not the same is the *path*. `Bash` runs with `--workdir /workspace`,
//! so every path it prints, and every path a compiler error inside the
//! container names, is rooted at `/workspace`. A model that reads `Bash` output
//! and hands one of those paths back to `Read` must get the file it just saw.
//! Before this existed it got "No such file or directory" on Linux, or a path
//! that could not even be parsed on Windows.
//!
//! That is the whole job here: same bytes, translated names.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use archon_tools::filesystem::{FileMeta, FileSystem, LocalFs};

/// Where the workspace bind mount lands inside the container.
///
/// Matches `workspace_mount_args` in `super::exec`; the two must agree, and a
/// test below pins them together rather than trusting the comment.
pub(crate) const CONTAINER_WORKSPACE: &str = "/workspace";

/// The container's scratch tmpfs, when `workspace_access = "scratch"`.
///
/// Lives only inside the container and is discarded with it, so it has no host
/// path at all.
const CONTAINER_SCRATCH: &str = "/scratch";

#[derive(Debug)]
pub struct DockerFs {
    working_dir: PathBuf,
    host: Arc<dyn FileSystem>,
    /// The `sandbox.workspace_access` this world was built for.
    ///
    /// Only `"rw"` mounts `/workspace` writable; `"ro"` (the default) and
    /// `"scratch"` both mount it `readonly` — see `workspace_mount_args` in
    /// `super::exec`, which computes exactly `workspace_access != "rw"`.
    workspace_access: String,
    /// Workspace-relative paths re-mounted writable over a read-only mount.
    ///
    /// `sandbox.docker.writable_paths`. Carried here for the same reason the
    /// access mode is: a gate that ignored them would refuse file-tool writes
    /// to a directory the container itself accepts, which is a different lie
    /// from the one being fixed but still a lie.
    writable_paths: Vec<String>,
    /// The workspace root as `path_guard` will have spelled it.
    ///
    /// A tool hands this filesystem a *canonicalised* host path, because
    /// `path_guard` canonicalises before it permits a write. On macOS a
    /// workspace under `/var` canonicalises to `/private/var`, and a
    /// containment check against the configured spelling alone would then say
    /// "outside the workspace" about every file in it — turning the gate off
    /// for exactly the paths that reach it.
    canonical_working_dir: PathBuf,
}

impl DockerFs {
    /// The workspace filesystem with **no write gate**.
    ///
    /// Preserved verbatim for callers that have no access mode to hand. It does
    /// not enforce the read-only mount — see
    /// [`with_workspace_access`](Self::with_workspace_access), which is the
    /// constructor a sandbox should use.
    #[must_use]
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self::with_workspace_access(working_dir, "rw", &[])
    }

    /// The workspace filesystem that answers writes the way the mount does.
    ///
    /// `DockerFs` is not a container filesystem: it translates a container path
    /// back to a host path and hands the operation to [`LocalFs`], because the
    /// bind mount means both names address the same bytes. That reasoning holds
    /// for reads and breaks for writes. `workspace_access = "ro"` mounts
    /// `/workspace` read-only, so `Bash` cannot write it — while `Write`,
    /// `Edit`, `ApplyPatch` and `LargeEdit` went around the mount entirely and
    /// changed the host disk. The setting read as enforced and governed only
    /// the shell.
    ///
    /// Given the mode, this refuses those writes with the setting named, so the
    /// filesystem and the shell give the same answer. Reads are untouched: the
    /// mount permits them and so does this.
    #[must_use]
    pub fn with_workspace_access(
        working_dir: impl Into<PathBuf>,
        workspace_access: &str,
        writable_paths: &[String],
    ) -> Self {
        let working_dir = working_dir.into();
        let canonical_working_dir = working_dir
            .canonicalize()
            .unwrap_or_else(|_| working_dir.clone());
        Self {
            working_dir,
            host: Arc::new(LocalFs),
            workspace_access: workspace_access.to_string(),
            writable_paths: writable_paths.to_vec(),
            canonical_working_dir,
        }
    }

    /// Refuse a write the container's own mount would refuse.
    ///
    /// `host` is the already-translated path, so the check is about the file
    /// that would actually change. A path outside the workspace is left alone:
    /// it is not under the mount at all, and bounding it is the host path
    /// guard's job, not this one's.
    fn ensure_writable(&self, requested: &Path, host: &Path, operation: &str) -> io::Result<()> {
        if self.workspace_access == "rw" {
            return Ok(());
        }
        let inside = host
            .strip_prefix(&self.working_dir)
            .or_else(|_| host.strip_prefix(&self.canonical_working_dir));
        let Ok(relative) = inside else {
            return Ok(());
        };
        if self
            .writable_paths
            .iter()
            .any(|writable| is_under(relative, writable))
        {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "cannot {operation} {}: the sandbox workspace is mounted read-only because \
                 sandbox.workspace_access = \"{}\". The container refuses this write, and \
                 performing it on the host copy instead would change a file the sandboxed \
                 shell is not allowed to change. Set sandbox.workspace_access = \"rw\", or \
                 list the path under sandbox.docker.writable_paths.",
                requested.display(),
                self.workspace_access
            ),
        ))
    }

    /// The host path for a path the model may have taken from `Bash` output.
    ///
    /// A path already rooted at the host working directory is returned
    /// unchanged, so the common case — the model using the paths it was given
    /// by `Read` and `Glob` — costs nothing and cannot be mangled.
    fn to_host(&self, path: &Path) -> io::Result<PathBuf> {
        let text = path.to_string_lossy().replace('\\', "/");

        if text == CONTAINER_SCRATCH || text.starts_with(&format!("{CONTAINER_SCRATCH}/")) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{} is inside the container's scratch tmpfs and has no host path; \
                     it is discarded when the container exits",
                    path.display()
                ),
            ));
        }

        if text == CONTAINER_WORKSPACE {
            return Ok(self.working_dir.clone());
        }

        let Some(relative) = text.strip_prefix(&format!("{CONTAINER_WORKSPACE}/")) else {
            return Ok(path.to_path_buf());
        };

        // A container path is the model repeating what the container told it,
        // so `..` in it is not an attack so much as a mistake — but it would
        // still escape the mount, which is the one thing the mount exists to
        // prevent. Refuse rather than resolve.
        let mut translated = self.working_dir.clone();
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(part) => translated.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{} leaves the workspace mount", path.display()),
                    ));
                }
            }
        }
        Ok(translated)
    }

    /// The container path for a host path, for results the model will see.
    ///
    /// `Glob` returns paths the model may paste straight into a `Bash`
    /// command, and a host path is meaningless in the container.
    fn to_container(&self, path: &Path) -> PathBuf {
        let Ok(relative) = path.strip_prefix(&self.working_dir) else {
            return path.to_path_buf();
        };
        let mut text = CONTAINER_WORKSPACE.to_string();
        for component in relative.components() {
            if let Component::Normal(part) = component {
                text.push('/');
                text.push_str(&part.to_string_lossy());
            }
        }
        PathBuf::from(text)
    }
}

#[async_trait::async_trait]
impl FileSystem for DockerFs {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.host.read(&self.to_host(path)?).await
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let host = self.to_host(path)?;
        self.ensure_writable(path, &host, "write")?;
        self.host.write(&host, contents).await
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let host = self.to_host(path)?;
        self.ensure_writable(path, &host, "create directory")?;
        self.host.create_dir_all(&host).await
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        self.host.metadata(&self.to_host(path)?).await
    }

    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let entries = self.host.read_dir(&self.to_host(path)?).await?;
        Ok(entries
            .into_iter()
            .map(|entry| self.to_container(&entry))
            .collect())
    }

    async fn remove_file(&self, path: &Path) -> io::Result<()> {
        let host = self.to_host(path)?;
        self.ensure_writable(path, &host, "remove")?;
        self.host.remove_file(&host).await
    }

    /// Both ends are checked: a rename removes `from` as surely as it creates
    /// `to`, so guarding only the destination would let a read-only workspace
    /// be emptied one move at a time.
    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let host_from = self.to_host(from)?;
        let host_to = self.to_host(to)?;
        self.ensure_writable(from, &host_from, "rename")?;
        self.ensure_writable(to, &host_to, "rename onto")?;
        self.host.rename(&host_from, &host_to).await
    }

    /// Container paths are this world's own naming, and the mount bounds them.
    ///
    /// `to_host` already refuses anything that would climb out of the mount, so
    /// a `/workspace/...` path is inside the workspace by construction and the
    /// host guard has nothing to add — it can only get it wrong, since the path
    /// does not exist on the host under that name.
    ///
    /// A host path is left to the host guard: `to_host` passes those through
    /// unchanged, so admitting them here would skip the working-directory check
    /// they still need.
    fn admit_world_path(&self, path: &Path) -> Option<io::Result<PathBuf>> {
        let text = path.to_string_lossy().replace('\\', "/");
        let is_container_path = text == CONTAINER_WORKSPACE
            || text.starts_with(&format!("{CONTAINER_WORKSPACE}/"))
            || text == CONTAINER_SCRATCH
            || text.starts_with(&format!("{CONTAINER_SCRATCH}/"));
        if !is_container_path {
            return None;
        }
        // Resolved for its verdict, then discarded: the operations translate
        // for themselves, and handing back a host path here would defeat
        // `to_container` on the way out.
        Some(self.to_host(path).map(|_| path.to_path_buf()))
    }

    /// The container mounts whichever directory the request names, so a child
    /// running in a worktree must translate against *that* tree.
    fn rerooted(self: Arc<Self>, working_dir: &Path) -> Arc<dyn FileSystem> {
        if working_dir == self.working_dir {
            return self;
        }
        // The access mode travels with the reroot. A child that inherited a
        // permissive filesystem would be the bypass with one extra step in it.
        Arc::new(Self::with_workspace_access(
            working_dir,
            &self.workspace_access,
            &self.writable_paths,
        ))
    }

    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
        let matched = self.host.glob(&self.to_host(base)?, pattern).await?;
        Ok(matched
            .into_iter()
            .map(|entry| self.to_container(&entry))
            .collect())
    }
}

/// Whether a workspace-relative path is at or below a `writable_paths` entry.
///
/// Compared component by component so `targeted/` is not matched by a plain
/// string prefix of `target`, and so the `/` separators these entries are
/// written with survive on a host that spells paths with `\\`.
fn is_under(relative: &Path, writable: &str) -> bool {
    let mut wanted = writable
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .peekable();
    if wanted.peek().is_none() {
        return false;
    }
    let mut actual = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        });
    for part in wanted {
        match actual.next() {
            Some(have) if have == part => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "fs_readonly_tests.rs"]
mod readonly_tests;
