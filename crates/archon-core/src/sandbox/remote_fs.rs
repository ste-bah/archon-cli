//! The filesystem of a world that is only reachable over a command transport
//! (#201 Phase 2, shared by ssh `remote` and openshell `remote`).
//!
//! Docker can answer filesystem questions locally because the bind mount means
//! the host and the container hold the same bytes. ssh in `remote` mode and
//! openshell in `remote` mode hold the tree somewhere else entirely, and the
//! only channel Archon has to that tree is the one `execute_bash` already
//! uses: run a command, read its output. So the filesystem is built out of
//! commands — `cat`, `stat`, `find`, `mv`, `rm`, `mkdir` — sent down that same
//! channel by the backend's own transport.
//!
//! Two properties drive every decision below.
//!
//! **Payloads are base64 on the wire.** Not for binary safety alone — a pipe
//! carries bytes fine — but because the far side runs `/bin/bash -lc`, and a
//! login shell sources a profile that is free to print. Text mixed into a
//! `cat` would silently corrupt the file the model then edits. Base64 makes
//! that corruption *detectable*: stray profile output is not in the alphabet,
//! so the read fails loudly instead of returning plausible garbage.
//!
//! **Nothing here reports success it did not observe.** A write is confirmed
//! by the far side echoing the resulting byte count, which is compared against
//! what was sent; a transport that quietly drops stdin produces an error, not
//! an empty file and an `Ok(())`.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use archon_tools::filesystem::{FileMeta, FileSystem};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command as TokioCommand;

/// How long one filesystem command may take on the far side.
///
/// Generous compared to a `stat`, because the same budget covers streaming a
/// large file back through the transport on a slow link.
pub(crate) const REMOTE_FS_TIMEOUT_MS: u64 = 120_000;

#[path = "remote_fs_scripts.rs"]
mod scripts;

pub(crate) use scripts::{
    EXIT_NO_BASE64, EXIT_NO_GLOBSTAR, create_dir_all_script, glob_script, metadata_script,
    read_dir_script, read_script, remove_file_script, rename_script, validate_glob_pattern,
    write_script,
};

/// One command's raw result, before any interpretation.
#[derive(Debug, Clone)]
pub(crate) struct RemoteOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The backend's own way of running a command in its world.
///
/// Deliberately the *whole* contract: implementors reuse the argument builder
/// `execute_bash` uses, so there is one transport per backend and not two.
#[async_trait::async_trait]
pub(crate) trait RemoteExec: Send + Sync + std::fmt::Debug {
    /// Run `script` under the far side's `/bin/bash`, feeding it `stdin`.
    async fn run(&self, script: &str, stdin: &[u8]) -> io::Result<RemoteOutput>;

    /// Prefix for error messages, e.g. `"ssh sandbox"`.
    fn label(&self) -> &'static str;
}

/// Translation between the host's workspace path and the far side's.
///
/// The model's paths come from wherever it last saw one — `Glob` output, a
/// compiler error, its own memory of the host working directory. A remote
/// world numbers those paths differently, and a path that is not translated is
/// a "No such file or directory" at best and the wrong file at worst.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceMap {
    host_root: PathBuf,
    remote_root: String,
}

impl WorkspaceMap {
    pub(crate) fn new(host_root: impl Into<PathBuf>, remote_root: impl Into<String>) -> Self {
        let mut remote_root = remote_root.into().trim().to_string();
        while remote_root.len() > 1 && remote_root.ends_with('/') {
            remote_root.pop();
        }
        Self {
            host_root: host_root.into(),
            remote_root,
        }
    }

    /// The far side's path for a path the model used.
    ///
    /// Host-workspace paths are rewritten; a path that is already absolute in
    /// the far side's terms passes through, so the model can name `/tmp` or
    /// `/etc` there without the mapping guessing at it.
    /// The same remote root, reached from a different host directory.
    pub(crate) fn rerooted(&self, host_root: impl Into<PathBuf>) -> Self {
        Self {
            host_root: host_root.into(),
            remote_root: self.remote_root.clone(),
        }
    }

    pub(crate) fn to_remote(&self, path: &Path) -> io::Result<String> {
        let text = path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is not valid UTF-8 and cannot be named on the sandbox world's command line",
                    path.display()
                ),
            )
        })?;
        if text.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a sandbox path must not contain NUL",
            ));
        }

        if let Ok(relative) = path.strip_prefix(&self.host_root) {
            let mut out = self.remote_root.clone();
            for component in relative.components() {
                match component {
                    Component::Normal(part) => {
                        let part = part.to_str().ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "a workspace path component is not valid UTF-8",
                            )
                        })?;
                        out.push('/');
                        out.push_str(part);
                    }
                    Component::CurDir => {}
                    // The mount exists to bound what the sandbox can touch, so
                    // a `..` that walks out of it is refused rather than
                    // resolved — even when it is the model repeating a path it
                    // was given rather than trying anything.
                    Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("{} leaves the sandbox workspace", path.display()),
                        ));
                    }
                }
            }
            return Ok(out);
        }

        if text.starts_with('/') {
            return Ok(text.to_string());
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is neither inside the workspace ({}) nor an absolute path in the sandbox world",
                path.display(),
                self.host_root.display()
            ),
        ))
    }
}

/// Spawn one transport process, feed it `stdin`, and collect raw bytes.
///
/// Separate from `execute_bash`'s spawn because that one nulls stdin and
/// lossily stringifies the output: correct for a shell tool, wrong for a file.
/// The hardening it applies — kill on drop, its own process group — is kept.
pub(crate) async fn run_transport_process(
    mut cmd: TokioCommand,
    stdin: &[u8],
    timeout_ms: u64,
    binary: &str,
) -> io::Result<RemoteOutput> {
    cmd.stdin(if stdin.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|error| io::Error::other(format!("failed to spawn {binary}: {error}")))?;

    // Written concurrently with the wait: a payload larger than the pipe
    // buffer would otherwise deadlock against a far side that is talking back.
    let pipe = child.stdin.take();
    let payload = stdin.to_vec();
    let writer = async move {
        if let Some(mut pipe) = pipe {
            pipe.write_all(&payload).await?;
            pipe.shutdown().await?;
        }
        Ok::<(), io::Error>(())
    };

    let waited = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        // A write error here is usually the far side having already exited;
        // its status and stderr say why, and they are the better diagnosis.
        let (write_result, output) = tokio::join!(writer, child.wait_with_output());
        if let Err(error) = write_result {
            tracing::debug!("{binary} sandbox filesystem stdin: {error}");
        }
        output
    })
    .await;

    match waited {
        Ok(Ok(output)) => Ok(RemoteOutput {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        Ok(Err(error)) => Err(io::Error::other(format!(
            "{binary} command failed: {error}"
        ))),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{binary} filesystem command timed out after {timeout_ms}ms"),
        )),
    }
}

/// A [`FileSystem`] made of commands run by `T`.
#[derive(Debug)]
pub(crate) struct RemoteFs<T> {
    exec: T,
    map: WorkspaceMap,
}

impl<T: RemoteExec> RemoteFs<T> {
    pub(crate) fn new(exec: T, map: WorkspaceMap) -> Self {
        Self { exec, map }
    }

    /// The far side's stdout, or the far side's own reason for refusing.
    fn succeeded(&self, operation: &str, path: &str, out: &RemoteOutput) -> io::Result<()> {
        if out.status == Some(0) {
            return Ok(());
        }
        let label = self.exec.label();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail: String = stderr.trim().chars().take(400).collect();
        let kind = match out.status {
            Some(EXIT_NO_BASE64 | EXIT_NO_GLOBSTAR) => io::ErrorKind::Unsupported,
            _ if detail.to_ascii_lowercase().contains("no such file") => io::ErrorKind::NotFound,
            _ if detail.to_ascii_lowercase().contains("permission denied") => {
                io::ErrorKind::PermissionDenied
            }
            _ if detail.to_ascii_lowercase().contains("file exists") => {
                io::ErrorKind::AlreadyExists
            }
            _ => io::ErrorKind::Other,
        };
        let status = out
            .status
            .map_or_else(|| "no exit code".to_string(), |code| format!("exit {code}"));
        Err(io::Error::new(
            kind,
            format!("{label}: {operation} {path} failed ({status}): {detail}"),
        ))
    }

    fn decode(&self, stdout: &[u8]) -> io::Result<Vec<u8>> {
        let packed: Vec<u8> = stdout
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect();
        BASE64.decode(&packed).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: expected base64 from the sandbox world but could not decode it ({error}); \
                     a shell profile on the far side may be printing to stdout",
                    self.exec.label()
                ),
            )
        })
    }

    /// Split a NUL-separated listing, refusing to guess at non-UTF-8 names.
    fn split_entries(&self, raw: &[u8], prefix: Option<&str>) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for chunk in raw.split(|byte| *byte == 0) {
            if chunk.is_empty() {
                continue;
            }
            let name = std::str::from_utf8(chunk).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{}: the sandbox world listed a name that is not valid UTF-8",
                        self.exec.label()
                    ),
                )
            })?;
            out.push(match prefix {
                Some(base) => PathBuf::from(format!("{}/{name}", base.trim_end_matches('/'))),
                None => PathBuf::from(name),
            });
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl<T: RemoteExec + Clone + 'static> FileSystem for RemoteFs<T> {
    /// Only the host side of the mapping moves.
    ///
    /// The far side is fixed: `remote` mode `cd`s to the configured
    /// `remote_workdir` for every command, whatever directory the request
    /// names. So a child running in a worktree is still looking at the same
    /// remote tree — what changes is which host paths translate into it, and
    /// without this the child's own paths would fail to translate at all.
    fn rerooted(self: std::sync::Arc<Self>, working_dir: &Path) -> std::sync::Arc<dyn FileSystem> {
        if self.map.host_root == working_dir {
            return self;
        }
        std::sync::Arc::new(RemoteFs {
            exec: self.exec.clone(),
            map: self.map.rerooted(working_dir),
        })
    }

    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let remote = self.map.to_remote(path)?;
        let out = self.exec.run(&read_script(&remote), &[]).await?;
        self.succeeded("read", &remote, &out)?;
        self.decode(&out.stdout)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let remote = self.map.to_remote(path)?;
        let payload = BASE64.encode(contents);
        let out = self
            .exec
            .run(&write_script(&remote), payload.as_bytes())
            .await?;
        self.succeeded("write", &remote, &out)?;

        let reported = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>();
        match reported {
            Ok(len) if len == contents.len() as u64 => Ok(()),
            Ok(len) => Err(io::Error::other(format!(
                "{}: wrote {} bytes to {remote} but the sandbox world reports {len}; \
                 the transport did not deliver the whole file",
                self.exec.label(),
                contents.len()
            ))),
            Err(_) => Err(io::Error::other(format!(
                "{}: {remote} was written but the sandbox world did not report its size, \
                 so the write could not be confirmed",
                self.exec.label()
            ))),
        }
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let remote = self.map.to_remote(path)?;
        let out = self.exec.run(&create_dir_all_script(&remote), &[]).await?;
        self.succeeded("create_dir_all", &remote, &out)
    }

    async fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        let remote = self.map.to_remote(path)?;
        let out = self.exec.run(&metadata_script(&remote), &[]).await?;
        self.succeeded("metadata", &remote, &out)?;
        parse_stat(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: could not read a size and mtime out of the stat output for {remote}",
                    self.exec.label()
                ),
            )
        })
    }

    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let remote = self.map.to_remote(path)?;
        let out = self.exec.run(&read_dir_script(&remote), &[]).await?;
        self.succeeded("read_dir", &remote, &out)?;
        let raw = self.decode(&out.stdout)?;
        self.split_entries(&raw, None)
    }

    async fn remove_file(&self, path: &Path) -> io::Result<()> {
        let remote = self.map.to_remote(path)?;
        let out = self.exec.run(&remove_file_script(&remote), &[]).await?;
        self.succeeded("remove_file", &remote, &out)
    }

    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let from = self.map.to_remote(from)?;
        let to = self.map.to_remote(to)?;
        let out = self.exec.run(&rename_script(&from, &to), &[]).await?;
        self.succeeded("rename", &from, &out)
    }

    async fn glob(&self, base: &Path, pattern: &str) -> io::Result<Vec<PathBuf>> {
        validate_glob_pattern(pattern)?;
        let remote = self.map.to_remote(base)?;
        let out = self.exec.run(&glob_script(&remote, pattern), &[]).await?;
        self.succeeded("glob", &remote, &out)?;
        let raw = self.decode(&out.stdout)?;
        self.split_entries(&raw, Some(&remote))
    }
}

/// `"{len} {mtime_seconds} {type}"` from either stat dialect.
pub(crate) fn parse_stat(text: &str) -> Option<FileMeta> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut parts = line.split_whitespace();
    let len = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<i64>().ok()?;
    let kind = parts.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    Some(FileMeta {
        len,
        // A negative mtime is a pre-epoch file; `modified_nanos` is unsigned,
        // so report it as unknown rather than wrapping it into the far future.
        modified_nanos: u64::try_from(seconds)
            .ok()
            .map(|secs| u128::from(secs) * 1_000_000_000),
        is_dir: kind.starts_with("directory"),
    })
}

#[cfg(test)]
#[path = "remote_fs_tests.rs"]
mod tests;

/// The scripts against a real bash, which needs a POSIX host to run on.
#[cfg(all(test, unix))]
#[path = "remote_fs_shell_tests.rs"]
mod shell_tests;
