use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerConfig {
    pub enabled: bool,
    pub binary: String,
    pub image: String,
    pub network: String,
    pub memory_limit: Option<String>,
    pub cpu_limit: Option<String>,
    pub writable_paths: Vec<String>,
    pub env_allowlist: Vec<String>,
    pub privileged: bool,
    pub mount_docker_socket: bool,
    pub mount_home: bool,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: "docker".into(),
            image: "ubuntu:24.04".into(),
            network: "disabled".into(),
            memory_limit: Some("2g".into()),
            cpu_limit: Some("2".into()),
            writable_paths: Vec::new(),
            env_allowlist: Vec::new(),
            privileged: false,
            mount_docker_socket: false,
            mount_home: false,
        }
    }
}

impl DockerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.binary.trim().is_empty() {
            return Err("sandbox.docker.binary must not be empty".into());
        }
        if self.image.trim().is_empty() {
            return Err("sandbox.docker.image must not be empty".into());
        }
        match self.network.as_str() {
            "disabled" | "limited" | "enabled" => Ok(()),
            other => Err(format!(
                "sandbox.docker.network must be disabled, limited, or enabled, got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerProbe {
    pub found: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl DockerProbe {
    pub fn missing(error: impl Into<String>) -> Self {
        Self {
            found: false,
            version: None,
            error: Some(error.into()),
        }
    }

    pub fn found(version: impl Into<String>) -> Self {
        Self {
            found: true,
            version: Some(version.into()),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerDoctorStatus {
    Disabled,
    ReadyDetectOnly,
    MissingBinary,
    UnsafeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerDoctorReport {
    pub status: DockerDoctorStatus,
    pub binary: String,
    pub version: Option<String>,
    pub findings: Vec<String>,
}

pub fn probe_docker(binary: &str) -> DockerProbe {
    match Command::new(binary).arg("--version").output() {
        Ok(output) => {
            let version = crate::sandbox::first_non_empty_line(&output.stdout)
                .or_else(|| crate::sandbox::first_non_empty_line(&output.stderr))
                .unwrap_or_else(|| "present (version unavailable)".into());
            DockerProbe::found(version)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            DockerProbe::missing(format!("{binary} not found on PATH"))
        }
        Err(err) => DockerProbe {
            found: false,
            version: None,
            error: Some(format!("{binary} probe failed: {err}")),
        },
    }
}

pub fn docker_doctor_report(config: &DockerConfig, probe: DockerProbe) -> DockerDoctorReport {
    let mut findings = Vec::new();
    findings.push("execution backend is detect-only in this release slice".into());
    findings.push("provider credentials, SSH agents, Git credentials, and host home mounts are not passed by default".into());

    let status = if config.privileged || config.mount_docker_socket || config.mount_home {
        findings.push(
            "unsafe config: privileged mode, Docker socket mount, or home mount is enabled".into(),
        );
        DockerDoctorStatus::UnsafeConfig
    } else if !config.enabled {
        findings.push("Docker backend is disabled in config".into());
        DockerDoctorStatus::Disabled
    } else if !probe.found {
        findings.push(
            probe
                .error
                .clone()
                .unwrap_or_else(|| "Docker binary was not found".into()),
        );
        DockerDoctorStatus::MissingBinary
    } else {
        findings.push(format!("image: {}", config.image));
        findings.push(format!("network: {}", config.network));
        findings.push(format!(
            "writable paths: {}",
            if config.writable_paths.is_empty() {
                "none".into()
            } else {
                config.writable_paths.join(", ")
            }
        ));
        DockerDoctorStatus::ReadyDetectOnly
    };

    DockerDoctorReport {
        status,
        binary: config.binary.clone(),
        version: probe.version,
        findings,
    }
}

pub fn render_docker_doctor_report(report: &DockerDoctorReport) -> String {
    let status = match report.status {
        DockerDoctorStatus::Disabled => "disabled",
        DockerDoctorStatus::ReadyDetectOnly => "ready-detect-only",
        DockerDoctorStatus::MissingBinary => "missing-binary",
        DockerDoctorStatus::UnsafeConfig => "unsafe-config",
    };
    let version = report.version.as_deref().unwrap_or("unknown");
    let mut out = format!(
        "Sandbox doctor\nBackend: docker\nStatus: {status}\nBinary: {}\nVersion: {version}\n",
        report.binary
    );
    for finding in &report.findings {
        out.push_str("- ");
        out.push_str(finding);
        out.push('\n');
    }
    out.push_str(
        "Execution: disabled until the Docker sandbox backend is explicitly implemented\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_defaults_are_safe() {
        let cfg = DockerConfig::default();

        assert!(!cfg.enabled);
        assert_eq!(cfg.binary, "docker");
        assert_eq!(cfg.network, "disabled");
        assert!(!cfg.privileged);
        assert!(!cfg.mount_docker_socket);
        assert!(!cfg.mount_home);
        assert!(cfg.env_allowlist.is_empty());
    }

    #[test]
    fn doctor_flags_unsafe_docker_config() {
        let cfg = DockerConfig {
            enabled: true,
            privileged: true,
            mount_docker_socket: true,
            mount_home: true,
            ..DockerConfig::default()
        };

        let report = docker_doctor_report(&cfg, DockerProbe::found("Docker 27.0.0"));

        assert_eq!(report.status, DockerDoctorStatus::UnsafeConfig);
        assert!(render_docker_doctor_report(&report).contains("unsafe-config"));
    }
}
