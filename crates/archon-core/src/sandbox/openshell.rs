use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenShellConfig {
    pub enabled: bool,
    pub binary: String,
    pub workspace_mode: String,
    pub gateway: Option<String>,
    pub policy: Option<String>,
    pub providers: Vec<String>,
    pub gpu: bool,
    pub provider_injection: bool,
    pub host_shell_fallback: bool,
}

impl Default for OpenShellConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: "openshell".into(),
            workspace_mode: "mirror".into(),
            gateway: None,
            policy: None,
            providers: Vec::new(),
            gpu: false,
            provider_injection: false,
            host_shell_fallback: false,
        }
    }
}

impl OpenShellConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.binary.trim().is_empty() {
            return Err("sandbox.openshell.binary must not be empty".into());
        }
        match self.workspace_mode.as_str() {
            "mirror" | "remote" => Ok(()),
            other => Err(format!(
                "sandbox.openshell.workspace_mode must be mirror or remote, got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShellProbe {
    pub found: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl OpenShellProbe {
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
pub enum OpenShellDoctorStatus {
    Disabled,
    ReadyDetectOnly,
    MissingBinary,
    MissingGateway,
    UnsafeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShellDoctorReport {
    pub status: OpenShellDoctorStatus,
    pub binary: String,
    pub version: Option<String>,
    pub findings: Vec<String>,
}

pub fn probe_openshell(binary: &str) -> OpenShellProbe {
    match Command::new(binary).arg("--version").output() {
        Ok(output) => {
            let version = crate::sandbox::first_non_empty_line(&output.stdout)
                .or_else(|| crate::sandbox::first_non_empty_line(&output.stderr))
                .unwrap_or_else(|| "present (version unavailable)".into());
            OpenShellProbe::found(version)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            OpenShellProbe::missing(format!("{binary} not found on PATH"))
        }
        Err(err) => OpenShellProbe {
            found: false,
            version: None,
            error: Some(format!("{binary} probe failed: {err}")),
        },
    }
}

pub fn openshell_doctor_report(
    config: &OpenShellConfig,
    probe: OpenShellProbe,
) -> OpenShellDoctorReport {
    let mut findings = Vec::new();
    findings.push("execution backend is detect-only in this release slice".into());
    findings.push(
        "provider injection is disabled by default; Anthropic spoofing remains host-side".into(),
    );

    let status = if config.provider_injection || config.host_shell_fallback {
        findings.push("unsafe config: provider injection or host shell fallback is enabled".into());
        OpenShellDoctorStatus::UnsafeConfig
    } else if !config.enabled {
        findings.push("OpenShell backend is disabled in config".into());
        OpenShellDoctorStatus::Disabled
    } else if !probe.found {
        findings.push(
            probe
                .error
                .clone()
                .unwrap_or_else(|| "OpenShell binary was not found".into()),
        );
        OpenShellDoctorStatus::MissingBinary
    } else if config.workspace_mode == "remote"
        && config.gateway.as_deref().unwrap_or("").is_empty()
    {
        findings.push("remote workspace mode requires an explicit gateway".into());
        OpenShellDoctorStatus::MissingGateway
    } else {
        findings.push(format!(
            "workspace mode: {}; remote canonical workspace is not active unless mode=remote",
            config.workspace_mode
        ));
        OpenShellDoctorStatus::ReadyDetectOnly
    };

    OpenShellDoctorReport {
        status,
        binary: config.binary.clone(),
        version: probe.version,
        findings,
    }
}

pub fn render_openshell_doctor_report(report: &OpenShellDoctorReport) -> String {
    let status = match report.status {
        OpenShellDoctorStatus::Disabled => "disabled",
        OpenShellDoctorStatus::ReadyDetectOnly => "ready-detect-only",
        OpenShellDoctorStatus::MissingBinary => "missing-binary",
        OpenShellDoctorStatus::MissingGateway => "missing-gateway",
        OpenShellDoctorStatus::UnsafeConfig => "unsafe-config",
    };
    let version = report.version.as_deref().unwrap_or("unknown");
    let mut out = format!(
        "Sandbox doctor\nBackend: openshell\nStatus: {status}\nBinary: {}\nVersion: {version}\n",
        report.binary
    );
    for finding in &report.findings {
        out.push_str("- ");
        out.push_str(finding);
        out.push('\n');
    }
    out.push_str(
        "Execution: disabled until the OpenShell execution backend is explicitly implemented\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openshell_defaults_are_safe() {
        let cfg = OpenShellConfig::default();

        assert!(!cfg.enabled);
        assert_eq!(cfg.binary, "openshell");
        assert_eq!(cfg.workspace_mode, "mirror");
        assert!(!cfg.provider_injection);
        assert!(!cfg.host_shell_fallback);
    }

    #[test]
    fn doctor_fails_closed_when_binary_missing() {
        let cfg = OpenShellConfig {
            enabled: true,
            ..OpenShellConfig::default()
        };

        let report = openshell_doctor_report(&cfg, OpenShellProbe::missing("not installed"));

        assert_eq!(report.status, OpenShellDoctorStatus::MissingBinary);
        assert!(render_openshell_doctor_report(&report).contains("missing-binary"));
    }

    #[test]
    fn doctor_rejects_provider_injection_and_host_fallback() {
        let cfg = OpenShellConfig {
            enabled: true,
            provider_injection: true,
            host_shell_fallback: true,
            ..OpenShellConfig::default()
        };

        let report = openshell_doctor_report(&cfg, OpenShellProbe::found("openshell 1.0.0"));

        assert_eq!(report.status, OpenShellDoctorStatus::UnsafeConfig);
        assert!(render_openshell_doctor_report(&report).contains("unsafe-config"));
    }
}
