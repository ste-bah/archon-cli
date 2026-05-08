use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub enabled: bool,
    pub binary: String,
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: u16,
    pub key_file: Option<String>,
    pub workspace_mode: String,
    pub host_key_checking: bool,
    pub host_shell_fallback: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: "ssh".into(),
            host: None,
            user: None,
            port: 22,
            key_file: None,
            workspace_mode: "remote".into(),
            host_key_checking: true,
            host_shell_fallback: false,
        }
    }
}

impl SshConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.binary.trim().is_empty() {
            return Err("sandbox.ssh.binary must not be empty".into());
        }
        if self.port == 0 {
            return Err("sandbox.ssh.port must be greater than 0".into());
        }
        match self.workspace_mode.as_str() {
            "mirror" | "remote" => Ok(()),
            other => Err(format!(
                "sandbox.ssh.workspace_mode must be mirror or remote, got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshProbe {
    pub found: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl SshProbe {
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
pub enum SshDoctorStatus {
    Disabled,
    ReadyDetectOnly,
    MissingBinary,
    MissingTarget,
    UnsafeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshDoctorReport {
    pub status: SshDoctorStatus,
    pub binary: String,
    pub version: Option<String>,
    pub findings: Vec<String>,
}

pub fn probe_ssh(binary: &str) -> SshProbe {
    match Command::new(binary).arg("-V").output() {
        Ok(output) => {
            let version = crate::sandbox::first_non_empty_line(&output.stdout)
                .or_else(|| crate::sandbox::first_non_empty_line(&output.stderr))
                .unwrap_or_else(|| "present (version unavailable)".into());
            SshProbe::found(version)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            SshProbe::missing(format!("{binary} not found on PATH"))
        }
        Err(err) => SshProbe {
            found: false,
            version: None,
            error: Some(format!("{binary} probe failed: {err}")),
        },
    }
}

pub fn ssh_doctor_report(config: &SshConfig, probe: SshProbe) -> SshDoctorReport {
    let mut findings = Vec::new();
    findings.push("execution backend is detect-only in this release slice".into());
    findings.push(
        "SSH sandboxing is remote execution; local Docker remains the local isolation backend"
            .into(),
    );
    findings
        .push("host-key verification must stay enabled to preserve TOFU mismatch blocking".into());

    let status = if !config.host_key_checking || config.host_shell_fallback {
        findings.push(
            "unsafe config: host-key checking is disabled or host shell fallback is enabled".into(),
        );
        SshDoctorStatus::UnsafeConfig
    } else if !config.enabled {
        findings.push("SSH sandbox backend is disabled in config".into());
        SshDoctorStatus::Disabled
    } else if !probe.found {
        findings.push(
            probe
                .error
                .clone()
                .unwrap_or_else(|| "SSH binary was not found".into()),
        );
        SshDoctorStatus::MissingBinary
    } else if config.host.as_deref().unwrap_or("").trim().is_empty() {
        findings.push("enabled SSH sandbox requires sandbox.ssh.host".into());
        SshDoctorStatus::MissingTarget
    } else {
        findings.push(format!(
            "target: {}@{}:{}",
            config.user.as_deref().unwrap_or("root"),
            config.host.as_deref().unwrap_or("unknown"),
            config.port
        ));
        findings.push(format!("workspace mode: {}", config.workspace_mode));
        SshDoctorStatus::ReadyDetectOnly
    };

    SshDoctorReport {
        status,
        binary: config.binary.clone(),
        version: probe.version,
        findings,
    }
}

pub fn render_ssh_doctor_report(report: &SshDoctorReport) -> String {
    let status = match report.status {
        SshDoctorStatus::Disabled => "disabled",
        SshDoctorStatus::ReadyDetectOnly => "ready-detect-only",
        SshDoctorStatus::MissingBinary => "missing-binary",
        SshDoctorStatus::MissingTarget => "missing-target",
        SshDoctorStatus::UnsafeConfig => "unsafe-config",
    };
    let version = report.version.as_deref().unwrap_or("unknown");
    let mut out = format!(
        "Sandbox doctor\nBackend: ssh\nStatus: {status}\nBinary: {}\nVersion: {version}\n",
        report.binary
    );
    for finding in &report.findings {
        out.push_str("- ");
        out.push_str(finding);
        out.push('\n');
    }
    out.push_str("Execution: disabled until the SSH sandbox backend is explicitly implemented\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_defaults_are_safe() {
        let cfg = SshConfig::default();

        assert!(!cfg.enabled);
        assert_eq!(cfg.binary, "ssh");
        assert_eq!(cfg.port, 22);
        assert!(cfg.host_key_checking);
        assert!(!cfg.host_shell_fallback);
    }

    #[test]
    fn doctor_requires_enabled_target() {
        let cfg = SshConfig {
            enabled: true,
            ..SshConfig::default()
        };

        let report = ssh_doctor_report(&cfg, SshProbe::found("OpenSSH_9.6"));

        assert_eq!(report.status, SshDoctorStatus::MissingTarget);
        assert!(render_ssh_doctor_report(&report).contains("missing-target"));
    }

    #[test]
    fn doctor_rejects_unsafe_ssh_config() {
        let cfg = SshConfig {
            enabled: true,
            host_key_checking: false,
            host_shell_fallback: true,
            ..SshConfig::default()
        };

        let report = ssh_doctor_report(&cfg, SshProbe::found("OpenSSH_9.6"));

        assert_eq!(report.status, SshDoctorStatus::UnsafeConfig);
        assert!(render_ssh_doctor_report(&report).contains("unsafe-config"));
    }
}
