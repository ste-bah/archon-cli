//! Detect-only reporting for `archon sandbox doctor --backend docker`.
//!
//! A sibling file because `docker.rs` reached the 500-line ceiling, and this is
//! the seam with no share in execution: nothing here runs a container or is on
//! the path of a command.

use std::process::Command;

use super::DockerConfig;

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
    findings
        .push("doctor is detect-only; Bash execution routes through Docker when selected".into());
    findings.push("provider credentials, SSH agents, Git credentials, and host home mounts are not passed by default".into());

    let status = if let Err(error) = config.validate() {
        findings.push(format!("invalid config: {error}"));
        DockerDoctorStatus::UnsafeConfig
    } else if config.privileged || config.mount_docker_socket || config.mount_home {
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
    out.push_str("Execution: Bash routes through Docker when sandbox.backend=docker\n");
    out
}
