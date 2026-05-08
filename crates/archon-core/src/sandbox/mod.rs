use serde::{Deserialize, Serialize};

mod docker;
mod openshell;
mod ssh;

pub use docker::{
    DockerConfig, DockerDoctorReport, DockerDoctorStatus, DockerProbe, docker_doctor_report,
    probe_docker, render_docker_doctor_report,
};
pub use openshell::{
    OpenShellConfig, OpenShellDoctorReport, OpenShellDoctorStatus, OpenShellProbe,
    openshell_doctor_report, probe_openshell, render_openshell_doctor_report,
};
pub use ssh::{
    SshConfig, SshDoctorReport, SshDoctorStatus, SshProbe, probe_ssh, render_ssh_doctor_report,
    ssh_doctor_report,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub backend: String,
    pub docker: DockerConfig,
    pub ssh: SshConfig,
    pub openshell: OpenShellConfig,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: "disabled".into(),
            docker: DockerConfig::default(),
            ssh: SshConfig::default(),
            openshell: OpenShellConfig::default(),
        }
    }
}

impl SandboxConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.backend.as_str() {
            "disabled" | "logical" | "docker" | "ssh" | "openshell" => {}
            other => {
                return Err(format!(
                    "sandbox.backend must be disabled, logical, docker, ssh, or openshell, got \"{other}\""
                ));
            }
        }
        self.docker.validate()?;
        self.ssh.validate()?;
        self.openshell.validate()
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

            [openshell]
            enabled = true
            workspace_mode = "remote"
            gateway = "team-gateway"
            policy = "locked-down"
            providers = ["ssh"]
            gpu = true
            "#,
        )
        .unwrap();

        assert_eq!(cfg.backend, "openshell");
        assert!(cfg.openshell.enabled);
        assert_eq!(cfg.openshell.workspace_mode, "remote");
        assert_eq!(cfg.openshell.gateway.as_deref(), Some("team-gateway"));
        assert!(cfg.openshell.gpu);
        assert!(!cfg.openshell.provider_injection);
        assert!(!cfg.openshell.host_shell_fallback);
    }

    #[test]
    fn sandbox_config_deserializes_ssh_section() {
        let cfg: SandboxConfig = toml::from_str(
            r#"
            backend = "ssh"

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
        assert!(cfg.ssh.enabled);
        assert_eq!(cfg.ssh.host.as_deref(), Some("sandbox.example"));
        assert_eq!(cfg.ssh.user.as_deref(), Some("archon"));
        assert_eq!(cfg.ssh.port, 2222);
        assert_eq!(cfg.ssh.workspace_mode, "remote");
        assert!(cfg.ssh.host_key_checking);
        assert!(!cfg.ssh.host_shell_fallback);
    }
}
