//! The filesystem of an openshell sandbox's workspace (#201 Phase 2).
//!
//! openshell has three workspace modes and only one of them puts the durable
//! tree on the far side.
//!
//! - `mirror` — the host copy is the working tree; local operations are it.
//! - `upload` (the default) — every command is a *fresh* `sandbox create
//!   --upload {working_dir}:/sandbox --no-keep` (see `super::exec`). The host
//!   tree is uploaded again for each command and the sandbox is destroyed
//!   after it, so the host tree is what the next command will see and the only
//!   copy that outlives one command. Local operations are therefore the
//!   correct — and the only honest — answer here. The corollary is worth
//!   stating plainly: writes `Bash` makes *inside* an upload-mode sandbox are
//!   discarded with it. That is the backend's existing behaviour, not
//!   something a filesystem seam can repair, and #201's "write via Bash, read
//!   via Read" acceptance test cannot pass in this mode until the backend
//!   either keeps the sandbox or downloads the tree back.
//! - `remote` — the tree lives under `remote_workdir` on the gateway side and
//!   is reached the same way `execute_bash` reaches it.
//!
//! The remote transport reuses `super::exec::openshell_create_args` and
//! `super::apply_openshell_env_policy`, so the provider-injection and
//! credential-stripping rules that guard `Bash` guard file access too.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_permissions::sandbox::SandboxCommandRequest;
use archon_tools::filesystem::{FileSystem, LocalFs};
use tokio::process::Command as TokioCommand;

use super::exec::{openshell_create_args, remote_workdir};
use super::{OpenShellConfig, OpenShellSandboxBackend, apply_openshell_env_policy};
use crate::sandbox::remote_fs::{
    REMOTE_FS_TIMEOUT_MS, RemoteExec, RemoteFs, RemoteOutput, WorkspaceMap, run_transport_process,
};

/// The filesystem the openshell backend's world presents, or why it has none.
///
/// Runs the backend's own preflight first: a config `execute_bash` refuses to
/// route must not gain a working filesystem through the side door.
pub fn openshell_filesystem(
    config: &OpenShellConfig,
    working_dir: &Path,
) -> Result<Arc<dyn FileSystem>, String> {
    OpenShellSandboxBackend::new(config.clone()).safe_to_route()?;
    openshell_filesystem_for_mode(config, working_dir)
}

fn openshell_filesystem_for_mode(
    config: &OpenShellConfig,
    working_dir: &Path,
) -> Result<Arc<dyn FileSystem>, String> {
    match config.workspace_mode.as_str() {
        "mirror" | "upload" => Ok(Arc::new(LocalFs)),
        "remote" => {
            let remote_root = remote_workdir(config);
            // Paths are built under this root, and a relative root would
            // resolve against whatever directory the sandbox shell starts in.
            if !remote_root.starts_with('/') {
                return Err(format!(
                    "sandbox.openshell.remote_workdir must be an absolute path in the sandbox, got \"{remote_root}\""
                ));
            }
            Ok(Arc::new(RemoteFs::new(
                OpenShellTransport {
                    config: config.clone(),
                    working_dir: working_dir.to_path_buf(),
                },
                WorkspaceMap::new(working_dir, remote_root),
            )))
        }
        other => Err(format!(
            "sandbox.openshell.workspace_mode must be mirror, remote, or upload, got \"{other}\""
        )),
    }
}

#[derive(Debug, Clone)]
struct OpenShellTransport {
    config: OpenShellConfig,
    working_dir: PathBuf,
}

#[async_trait::async_trait]
impl RemoteExec for OpenShellTransport {
    async fn run(&self, script: &str, stdin: &[u8]) -> io::Result<RemoteOutput> {
        let args = openshell_fs_args(&self.config, &self.working_dir, script)
            .map_err(|error| io::Error::other(format!("openshell sandbox: {error}")))?;
        let mut cmd = TokioCommand::new(&self.config.binary);
        cmd.args(args);
        apply_openshell_env_policy(&mut cmd, &self.config);
        run_transport_process(cmd, stdin, REMOTE_FS_TIMEOUT_MS, "openshell").await
    }

    fn label(&self) -> &'static str {
        "openshell sandbox"
    }
}

fn openshell_fs_args(
    config: &OpenShellConfig,
    working_dir: &Path,
    script: &str,
) -> Result<Vec<String>, String> {
    openshell_create_args(
        config,
        &SandboxCommandRequest {
            command: script.to_string(),
            working_dir: working_dir.to_path_buf(),
            timeout_ms: REMOTE_FS_TIMEOUT_MS,
            // A file must come back whole; a shell tool's output budget would
            // truncate it into a corrupt one.
            max_output_bytes: usize::MAX,
            env: Vec::new(),
            // A file transfer belongs to no turn and no sandbox lifetime: this
            // backend builds a throwaway sandbox per command whatever the
            // scope, which is what `scope_support` says out loud.
            ..SandboxCommandRequest::default()
        },
    )
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
