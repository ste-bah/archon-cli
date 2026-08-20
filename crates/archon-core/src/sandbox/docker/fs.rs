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
}

impl DockerFs {
    #[must_use]
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
            host: Arc::new(LocalFs),
        }
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
        self.host.write(&self.to_host(path)?, contents).await
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.host.create_dir_all(&self.to_host(path)?).await
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
        self.host.remove_file(&self.to_host(path)?).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.host
            .rename(&self.to_host(from)?, &self.to_host(to)?)
            .await
    }

    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
        let matched = self.host.glob(&self.to_host(base)?, pattern).await?;
        Ok(matched
            .into_iter()
            .map(|entry| self.to_container(&entry))
            .collect())
    }
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
