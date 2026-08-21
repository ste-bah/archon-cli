use serde::{Deserialize, Serialize};

mod capability_gate;
mod docker;
mod openshell;
mod policy;
// Shared by the ssh and openshell `remote` modes: both hold the workspace on
// the far side of a command transport, and the commands that read and write it
// are the same commands. Only the transport differs.
mod remote_fs;
mod ssh;

pub use docker::{
    DockerConfig, DockerDoctorReport, DockerDoctorStatus, DockerFs, DockerProbe,
    DockerSandboxBackend, docker_doctor_report, probe_docker, render_docker_doctor_report,
};
pub use openshell::{
    OpenShellConfig, OpenShellDoctorReport, OpenShellDoctorStatus, OpenShellProbe,
    OpenShellSandboxBackend, openshell_doctor_report, openshell_filesystem, probe_openshell,
    render_openshell_doctor_report,
};
pub use policy::{SandboxBackendKind, SandboxPolicy};
pub use ssh::{
    SshConfig, SshDoctorReport, SshDoctorStatus, SshProbe, SshSandboxBackend, probe_ssh,
    render_ssh_doctor_report, ssh_doctor_report, ssh_filesystem,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub backend: String,
    pub mode: String,
    pub scope: String,
    pub workspace_access: String,
    pub docker: DockerConfig,
    pub ssh: SshConfig,
    pub openshell: OpenShellConfig,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: "disabled".into(),
            mode: "risky".into(),
            scope: "session".into(),
            workspace_access: "ro".into(),
            docker: DockerConfig::default(),
            ssh: SshConfig::default(),
            openshell: OpenShellConfig::default(),
        }
    }
}

impl SandboxConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.policy()?.validate()?;
        self.docker.validate()?;
        self.ssh.validate()?;
        self.openshell.validate()
    }

    pub fn backend_kind(&self) -> Result<SandboxBackendKind, String> {
        self.backend.parse()
    }

    pub fn policy(&self) -> Result<SandboxPolicy, String> {
        Ok(SandboxPolicy {
            backend: self.backend_kind()?,
            mode: self.mode.clone(),
            scope: self.scope.clone(),
            workspace_access: self.workspace_access.clone(),
        })
    }
}

/// The filesystem of the world this configuration executes in (#201 Phase 2).
///
/// `Ok(None)` means the host, and is the honest answer in more cases than it
/// looks: a disabled or `logical` backend never leaves the host, docker's
/// workspace is a bind mount of it, and both `mirror` modes assume the same
/// tree is visible on the far side. Only a genuinely remote workspace needs a
/// filesystem of its own.
///
/// An error here must fail session boot rather than degrade to the host. The
/// whole point is that the agent's reads and its shell agree about which tree
/// they are on; quietly handing back the host when the remote filesystem could
/// not be built would restore exactly the split this issue exists to close,
/// and would do it silently.
pub fn sandbox_filesystem(
    config: &SandboxConfig,
    working_dir: &std::path::Path,
) -> Result<Option<std::sync::Arc<dyn archon_tools::filesystem::FileSystem>>, String> {
    match config.backend_kind()? {
        SandboxBackendKind::Disabled | SandboxBackendKind::Logical => Ok(None),
        SandboxBackendKind::Docker => Ok(Some(std::sync::Arc::new(DockerFs::new(working_dir)))),
        SandboxBackendKind::Ssh => ssh_filesystem(&config.ssh, working_dir).map(Some),
        SandboxBackendKind::OpenShell => {
            openshell_filesystem(&config.openshell, working_dir).map(Some)
        }
    }
}

pub(crate) fn first_non_empty_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_config_deserializes_openshell_section() {
        let cfg: SandboxConfig = toml::from_str(
            r#"
            backend = "openshell"
            mode = "all"
            scope = "session"
            workspace_access = "rw"

            [openshell]
            enabled = true
            workspace_mode = "remote"
            gateway = "team-gateway"
            remote_workdir = "/workspace/team"
            policy = "locked-down"
            providers = ["ssh"]
            gpu = true
            "#,
        )
        .unwrap();

        assert_eq!(cfg.backend, "openshell");
        assert_eq!(cfg.backend_kind().unwrap(), SandboxBackendKind::OpenShell);
        assert_eq!(cfg.policy().unwrap().mode, "all");
        assert_eq!(cfg.policy().unwrap().workspace_access, "rw");
        assert!(cfg.openshell.enabled);
        assert_eq!(cfg.openshell.workspace_mode, "remote");
        assert_eq!(cfg.openshell.gateway.as_deref(), Some("team-gateway"));
        assert_eq!(
            cfg.openshell.remote_workdir.as_deref(),
            Some("/workspace/team")
        );
        assert!(cfg.openshell.gpu);
        assert!(!cfg.openshell.provider_injection);
        assert!(!cfg.openshell.host_shell_fallback);
    }

    #[test]
    fn sandbox_config_deserializes_ssh_section() {
        let cfg: SandboxConfig = toml::from_str(
            r#"
            backend = "ssh"
            mode = "all"

            [ssh]
            enabled = true
            host = "sandbox.example"
            user = "archon"
            port = 2222
            workspace_mode = "remote"
            "#,
        )
        .unwrap();

        assert_eq!(cfg.backend, "ssh");
        assert_eq!(cfg.backend_kind().unwrap(), SandboxBackendKind::Ssh);
        assert!(cfg.ssh.enabled);
        assert_eq!(cfg.ssh.host.as_deref(), Some("sandbox.example"));
        assert_eq!(cfg.ssh.user.as_deref(), Some("archon"));
        assert_eq!(cfg.ssh.port, 2222);
        assert_eq!(cfg.ssh.workspace_mode, "remote");
        assert!(cfg.ssh.host_key_checking);
        assert!(!cfg.ssh.host_shell_fallback);
    }

    /// A backend that never leaves the host must not pay for a filesystem it
    /// does not need — and `None` here is what keeps behaviour identical for
    /// everyone not running a sandbox.
    #[test]
    fn a_host_bound_backend_gets_no_filesystem_of_its_own() {
        for backend in ["disabled", "logical"] {
            let cfg = SandboxConfig {
                backend: backend.into(),
                ..SandboxConfig::default()
            };

            assert!(
                sandbox_filesystem(&cfg, std::path::Path::new("/tree"))
                    .expect("host-bound backends always resolve")
                    .is_none(),
                "{backend} should run on the host filesystem"
            );
        }
    }

    /// Docker must get a *translating* filesystem, not merely some filesystem.
    /// Asserting `is_some()` would pass if the factory handed back `LocalFs`,
    /// which is the bug: `Bash` reports `/workspace/...` and `Read` would then
    /// look for that path on the host.
    #[tokio::test]
    async fn docker_gets_a_filesystem_that_translates_container_paths() {
        use archon_tools::filesystem::FileSystem;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("in_workspace.txt"), "mounted").expect("write");
        let cfg = SandboxConfig {
            backend: "docker".into(),
            ..SandboxConfig::default()
        };

        let fs = sandbox_filesystem(&cfg, dir.path())
            .expect("docker resolves")
            .expect("docker has a world of its own");

        assert_eq!(
            fs.read(std::path::Path::new("/workspace/in_workspace.txt"))
                .await
                .expect("the path the container would name"),
            b"mounted"
        );
    }

    /// A remote backend that cannot be routed must fail loudly. Returning the
    /// host here would be the silent split this whole issue exists to close.
    #[test]
    fn an_unroutable_remote_backend_is_an_error_not_a_fallback_to_the_host() {
        let cfg = SandboxConfig {
            backend: "ssh".into(),
            ..SandboxConfig::default()
        };

        let error = sandbox_filesystem(&cfg, std::path::Path::new("/tree"))
            .expect_err("a disabled ssh backend cannot supply a workspace");

        assert!(!error.is_empty(), "the failure has to say why");
    }

    #[test]
    fn sandbox_config_rejects_unknown_backend() {
        let cfg = SandboxConfig {
            backend: "host".into(),
            ..SandboxConfig::default()
        };

        let error = cfg.validate().unwrap_err();

        assert!(error.contains("sandbox.backend"));
    }
}
