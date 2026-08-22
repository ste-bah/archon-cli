//! The filesystem of an ssh sandbox's workspace (#201 Phase 2).
//!
//! `workspace_mode` decides which world holds the tree, and that is the whole
//! choice this module makes:
//!
//! - `mirror` — the host copy *is* the working tree, so the local filesystem
//!   is the correct answer and routing a `cat` over ssh would only be slower.
//! - `remote` (the default) — the tree lives under `remote_workdir` on the far
//!   side. Before this existed, `Read` and `Grep` answered from the host while
//!   `Bash` ran there: the agent reasoned about one filesystem and executed
//!   against another, silently. Now both go down the same wire.
//!
//! The wire is `super::exec::ssh_command_args` — the same argument builder
//! `execute_bash` uses, so the hardening (`BatchMode`, `StrictHostKeyChecking`,
//! `ForwardAgent=no`, no environment forwarding) applies unchanged and there is
//! one ssh invocation shape in this backend, not two.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_permissions::sandbox::SandboxCommandRequest;
use archon_tools::filesystem::{FileSystem, LocalFs};
use tokio::process::Command as TokioCommand;

use super::exec::ssh_command_args;
use super::{SshConfig, SshSandboxBackend};
use crate::sandbox::remote_fs::{
    REMOTE_FS_TIMEOUT_MS, RemoteExec, RemoteFs, RemoteOutput, WorkspaceMap, run_transport_process,
};

/// The filesystem the ssh backend's world presents, or why it has none.
///
/// Fails closed through the backend's own preflight: a config that
/// `execute_bash` would refuse to route must not get a working filesystem
/// either, or `Read` would become a way around the refusal.
pub fn ssh_filesystem(
    config: &SshConfig,
    working_dir: &Path,
) -> Result<Arc<dyn FileSystem>, String> {
    SshSandboxBackend::new(config.clone()).safe_to_route()?;
    ssh_filesystem_for_mode(config, working_dir)
}

fn ssh_filesystem_for_mode(
    config: &SshConfig,
    working_dir: &Path,
) -> Result<Arc<dyn FileSystem>, String> {
    match config.workspace_mode.as_str() {
        "mirror" => Ok(Arc::new(LocalFs)),
        "remote" => {
            let remote_root = config
                .remote_workdir
                .as_deref()
                .map(str::trim)
                .filter(|workdir| !workdir.is_empty())
                .ok_or_else(|| {
                    "ssh sandbox remote mode requires sandbox.ssh.remote_workdir".to_string()
                })?;
            // Every command this builds names paths under this root, and a
            // relative root would resolve against whatever directory the
            // remote login shell happens to start in.
            if !remote_root.starts_with('/') {
                return Err(format!(
                    "sandbox.ssh.remote_workdir must be an absolute path on the remote host, got \"{remote_root}\""
                ));
            }
            Ok(Arc::new(RemoteFs::new(
                SshTransport {
                    config: config.clone(),
                    working_dir: working_dir.to_path_buf(),
                },
                WorkspaceMap::new(working_dir, remote_root),
            )))
        }
        other => Err(format!(
            "sandbox.ssh.workspace_mode must be mirror or remote, got \"{other}\""
        )),
    }
}

#[derive(Debug, Clone)]
struct SshTransport {
    config: SshConfig,
    /// Only reaches `ssh_command_args` as the mirror-mode working directory,
    /// which this transport never uses; carried so the request handed to the
    /// shared builder is the real one rather than a placeholder.
    working_dir: PathBuf,
}

#[async_trait::async_trait]
impl RemoteExec for SshTransport {
    async fn run(&self, script: &str, stdin: &[u8]) -> io::Result<RemoteOutput> {
        let args = ssh_fs_args(&self.config, &self.working_dir, script)
            .map_err(|error| io::Error::other(format!("ssh sandbox: {error}")))?;
        let mut cmd = TokioCommand::new(&self.config.binary);
        cmd.args(args);
        run_transport_process(cmd, stdin, REMOTE_FS_TIMEOUT_MS, "ssh").await
    }

    fn label(&self) -> &'static str {
        "ssh sandbox"
    }
}

fn ssh_fs_args(
    config: &SshConfig,
    working_dir: &Path,
    script: &str,
) -> Result<Vec<String>, String> {
    ssh_command_args(
        config,
        &SandboxCommandRequest {
            command: script.to_string(),
            working_dir: working_dir.to_path_buf(),
            timeout_ms: REMOTE_FS_TIMEOUT_MS,
            // A filesystem read must come back whole; truncating it to a shell
            // tool's output budget would hand the model a corrupt file.
            max_output_bytes: usize::MAX,
            env: Vec::new(),
            // A file transfer belongs to no turn and no sandbox lifetime: the
            // ssh backend's world is the remote host, which every command
            // reaches regardless.
            ..SandboxCommandRequest::default()
        },
    )
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
