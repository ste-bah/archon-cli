//! The filesystem of the execution world (#201 Phase 1).
//!
//! Archon has exactly one sandboxed execution path — `SandboxBackend::execute_bash` —
//! and it serves exactly one tool. Every other tool calls `std::fs` directly, so
//! under a sandbox the agent reads the *host* while `Bash` runs somewhere else.
//! The isolation backends defend against that the only way they can: by refusing
//! almost every tool by name.
//!
//! This trait is the other half of the pair. Point the filesystem and the
//! subprocess at the same world and the world-bound tools follow with no
//! per-tool forks. A backend that mounts the host tree (docker bind mounts)
//! answers with local operations; one that holds the tree elsewhere (ssh in
//! `remote` mode, openshell in `upload` mode) answers over its own transport.
//!
//! Operations are async because a remote world's are. [`LocalFs`] is the default
//! everywhere and does exactly what the call sites did before, so with no
//! sandbox configured behaviour is unchanged.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;

use crate::file_observation::FileVersion;

/// What a world reports about one path.
///
/// Deliberately smaller than `std::fs::Metadata`: a remote world may know a
/// size and a modification time and nothing else, and every consumer in the
/// workspace needs only these three facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub len: u64,
    /// Nanoseconds since the Unix epoch.
    ///
    /// `None` when the world does not report one — an FTP-like transport, or a
    /// filesystem whose times are unavailable. Callers must treat that as
    /// "unknown", never as "epoch": sorting glob results by a fabricated zero
    /// would silently reorder them.
    pub modified_nanos: Option<u128>,
    pub is_dir: bool,
}

/// The filesystem tools operate on.
///
/// Implementors own path resolution for their world. The `Path` given to these
/// methods is the path the *model* used, already guarded by `path_guard`; a
/// backend whose world numbers paths differently (a container mount point, a
/// remote workdir) translates on the way in and out.
#[async_trait::async_trait]
pub trait FileSystem: Send + Sync + std::fmt::Debug {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    async fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()>;

    async fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    async fn metadata(&self, path: &Path) -> io::Result<FileMeta>;

    /// Immediate children of `path`, in whatever order the world lists them.
    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;

    async fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Move `from` onto `to`, replacing it.
    ///
    /// Part of the trait because the write-to-temp-then-rename dance is how a
    /// notebook avoids being left half-written, and a caller that had to do it
    /// with `write` plus `remove_file` would lose the atomicity that makes it
    /// worth doing. Both paths are in this world; renaming across worlds is
    /// not a thing this expresses.
    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Decide a path the model named, when this world names paths its own way.
    ///
    /// `None` means "not one of mine": the caller applies the host path guard,
    /// which is what keeps `Read` and `Write` inside the working directory.
    /// `Some(Ok(path))` is a path this world recognises and vouches for —
    /// it is inside the workspace by construction, so the host guard must not
    /// be applied to it, because canonicalising `/workspace/src/main.rs` on the
    /// host fails and the tool would refuse a file that plainly exists.
    /// `Some(Err(..))` is a path this world recognises and refuses.
    ///
    /// A world that answers `None` everywhere behaves exactly as before this
    /// existed: its paths are host paths and the host guard bounds them. That is
    /// why the default is safe — it fails closed and visibly (a world path is
    /// rejected), never open.
    fn admit_world_path(&self, _path: &Path) -> Option<io::Result<PathBuf>> {
        None
    }

    /// The same world, rooted at `working_dir`.
    ///
    /// A subagent may run somewhere other than its parent — a worktree, or an
    /// explicit `cwd` — and its shell takes *that* directory as the workspace:
    /// docker mounts `ctx.working_dir`, not the session's. A filesystem left
    /// rooted at the parent's tree would then translate the child's own paths
    /// to the wrong files, silently, which is the split this seam exists to
    /// close — reintroduced exactly where Archon runs the most agents at once.
    ///
    /// Required rather than defaulted to `self`, because a default is right for
    /// a world with no root and quietly wrong for every world that has one, and
    /// the wrongness is invisible.
    fn rerooted(self: Arc<Self>, working_dir: &Path) -> Arc<dyn FileSystem>;

    /// Paths under `base` matching `pattern`, as absolute paths in this world.
    ///
    /// Required rather than defaulted to a `read_dir` walk, because the naive
    /// walk is a performance trap: `src/**/*.rs` must not descend `target/`,
    /// and only the world knows how to avoid it. The host answers with the
    /// `glob` crate, which prunes by pattern prefix; a container answers with
    /// one command in the container. A default would work and would quietly be
    /// the slow one everywhere it was not overridden.
    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>>;

    /// Whether anything is at `path`.
    ///
    /// Derived from [`metadata`](FileSystem::metadata) so a backend cannot
    /// answer this one from a different world than the one it reads from.
    async fn exists(&self, path: &Path) -> bool {
        self.metadata(path).await.is_ok()
    }

    async fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    /// This world's version token for `path`, or `None` when it is not there.
    ///
    /// Defaulted rather than required because deriving it from this world's own
    /// [`metadata`](FileSystem::metadata) is correct for every backend: the
    /// token then describes the same file the write will land on, which is the
    /// whole property #193's freshness guard needs and could not have off-host.
    /// A backend with something better — an etag, a content hash — overrides it.
    async fn version(&self, path: &Path) -> Option<FileVersion> {
        let meta = self.metadata(path).await.ok()?;
        Some(FileVersion::from_parts(meta.len, meta.modified_nanos))
    }
}

/// The host filesystem: what every call site did before this trait existed.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFs;

#[async_trait::async_trait]
impl FileSystem for LocalFs {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let path = path.to_path_buf();
        spawn_blocking_io(move || std::fs::read(&path)).await
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let path = path.to_path_buf();
        let contents = contents.to_vec();
        spawn_blocking_io(move || std::fs::write(&path, contents)).await
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        spawn_blocking_io(move || std::fs::create_dir_all(&path)).await
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        let path = path.to_path_buf();
        spawn_blocking_io(move || {
            let meta = std::fs::metadata(&path)?;
            Ok(FileMeta {
                len: meta.len(),
                modified_nanos: meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_nanos()),
                is_dir: meta.is_dir(),
            })
        })
        .await
    }

    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let path = path.to_path_buf();
        spawn_blocking_io(move || {
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&path)? {
                entries.push(entry?.path());
            }
            Ok(entries)
        })
        .await
    }

    async fn remove_file(&self, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        spawn_blocking_io(move || std::fs::remove_file(&path)).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let from = from.to_path_buf();
        let to = to.to_path_buf();
        spawn_blocking_io(move || std::fs::rename(&from, &to)).await
    }

    /// The host has no root to move: every path in it is already absolute.
    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }

    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
        let joined = base.join(pattern);
        spawn_blocking_io(move || {
            let entries = glob::glob(&joined.to_string_lossy())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
            let mut matched = Vec::new();
            for entry in entries {
                match entry {
                    Ok(path) => matched.push(path),
                    // A directory the walk could not enter is not a failed
                    // glob; the surrounding matches are still the answer.
                    Err(error) => tracing::debug!("glob entry error: {error}"),
                }
            }
            Ok(matched)
        })
        .await
    }
}

/// Run one blocking filesystem call off the async runtime's worker threads.
///
/// Tool execution happens on the same runtime that drives the provider stream
/// and the TUI; a synchronous read of a large file on a slow disk stalls both.
async fn spawn_blocking_io<T, F>(operation: F) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(format!(
            "filesystem task did not finish: {error}"
        ))),
    }
}

/// The process-wide local filesystem.
///
/// One allocation for the whole run: `ToolContext::fs` hands out a clone per
/// tool call, and minting a fresh `Arc` each time would be pure churn.
static LOCAL_FS: LazyLock<Arc<dyn FileSystem>> = LazyLock::new(|| Arc::new(LocalFs));

/// The filesystem to use when no sandbox has installed one.
#[must_use]
pub fn local_fs() -> Arc<dyn FileSystem> {
    Arc::clone(&LOCAL_FS)
}

#[cfg(test)]
#[path = "filesystem_tests.rs"]
mod tests;
