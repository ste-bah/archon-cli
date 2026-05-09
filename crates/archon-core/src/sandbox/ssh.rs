use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::time::Duration;

use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

mod exec;

use exec::{ssh_command_args, ssh_output_result};

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
    pub remote_workdir: Option<String>,
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
            remote_workdir: None,
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
    MissingWorkspace,
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
    findings.push("doctor is detect-only; Bash execution routes through SSH when selected".into());
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
    } else if config.workspace_mode == "remote"
        && config
            .remote_workdir
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        findings.push("remote workspace mode requires sandbox.ssh.remote_workdir".into());
        SshDoctorStatus::MissingWorkspace
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
        SshDoctorStatus::MissingWorkspace => "missing-workspace",
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
    out.push_str("Execution: Bash routes through SSH when sandbox.backend=ssh\n");
    out
}

#[derive(Debug, Clone)]
pub struct SshSandboxBackend {
    config: SshConfig,
}

impl SshSandboxBackend {
    pub fn new(config: SshConfig) -> Self {
        Self { config }
    }

    fn safe_to_route(&self) -> Result<(), String> {
        self.config.validate()?;
        if !self.config.host_key_checking {
            return Err("ssh sandbox refuses disabled host-key checking".into());
        }
        if self.config.host_shell_fallback {
            return Err("ssh sandbox refuses host shell fallback".into());
        }
        if !self.config.enabled {
            return Err("ssh sandbox backend is disabled".into());
        }
        if self.config.host.as_deref().unwrap_or("").trim().is_empty() {
            return Err("ssh sandbox requires sandbox.ssh.host".into());
        }
        if self.config.workspace_mode == "remote"
            && self
                .config
                .remote_workdir
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("ssh sandbox remote mode requires sandbox.ssh.remote_workdir".into());
        }
        let probe = probe_ssh(&self.config.binary);
        if !probe.found {
            return Err(probe
                .error
                .unwrap_or_else(|| "ssh binary was not found".into()));
        }
        Ok(())
    }
}

impl SandboxBackend for SshSandboxBackend {
    fn check(&self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
        self.safe_to_route()?;
        match tool {
            "Read" | "Glob" | "Grep" | "ToolSearch" | "TodoWrite" | "Sleep" => Ok(()),
            "Bash" | "Shell" => Ok(()),
            "Write" | "Edit" | "NotebookEdit" => Err(format!(
                "ssh sandbox: {tool} host-side file mutation is not supported"
            )),
            "WebFetch" | "WebSearch" => Err(format!(
                "ssh sandbox: {tool} host-side network access is not supported"
            )),
            "TaskCreate" | "TaskUpdate" | "Agent" => Err(format!(
                "ssh sandbox: {tool} agent spawning is not supported"
            )),
            other => Err(format!("ssh sandbox: unsupported tool {other}")),
        }
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async move { Some(self.execute_bash_inner(request).await) })
    }
}

impl SshSandboxBackend {
    async fn execute_bash_inner(&self, request: SandboxCommandRequest) -> SandboxCommandResult {
        if let Err(error) = self.safe_to_route() {
            return SandboxCommandResult {
                content: format!(
                    "SSH sandbox refused execution: {error}; no host shell fallback was used.\n"
                ),
                is_error: true,
            };
        }
        let args = match ssh_command_args(&self.config, &request) {
            Ok(args) => args,
            Err(error) => {
                return SandboxCommandResult {
                    content: format!(
                        "SSH sandbox refused execution: {error}; no host shell fallback was used.\n"
                    ),
                    is_error: true,
                };
            }
        };
        let mut cmd = TokioCommand::new(&self.config.binary);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                return SandboxCommandResult {
                    content: format!("Error: Failed to spawn ssh: {error}"),
                    is_error: true,
                };
            }
        };

        match tokio::time::timeout(
            Duration::from_millis(request.timeout_ms),
            child.wait_with_output(),
        )
        .await
        {
            Ok(Ok(output)) => ssh_output_result(output, request.max_output_bytes),
            Ok(Err(error)) => SandboxCommandResult {
                content: format!("Error: SSH command failed: {error}"),
                is_error: true,
            },
            Err(_) => SandboxCommandResult {
                content: format!(
                    "Error: SSH command timed out after {}ms",
                    request.timeout_ms
                ),
                is_error: true,
            },
        }
    }
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
        assert_eq!(cfg.remote_workdir, None);
        assert!(cfg.host_key_checking);
        assert!(!cfg.host_shell_fallback);
    }

    #[test]
    fn doctor_requires_remote_workdir_for_remote_mode() {
        let cfg = SshConfig {
            enabled: true,
            host: Some("sandbox.example".into()),
            ..SshConfig::default()
        };

        let report = ssh_doctor_report(&cfg, SshProbe::found("OpenSSH_9.6"));

        assert_eq!(report.status, SshDoctorStatus::MissingWorkspace);
        assert!(render_ssh_doctor_report(&report).contains("missing-workspace"));
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

    #[test]
    fn backend_requires_host_without_falling_back() {
        let backend = SshSandboxBackend::new(SshConfig {
            enabled: true,
            ..SshConfig::default()
        });

        let error = backend.check("Bash", &serde_json::json!({})).unwrap_err();

        assert!(error.contains("sandbox.ssh.host"));
    }

    #[tokio::test]
    async fn backend_execute_bash_returns_error_without_host_fallback() {
        let backend = SshSandboxBackend::new(SshConfig {
            enabled: true,
            host: Some("example.invalid".into()),
            ..SshConfig::default()
        });
        let result = backend
            .execute_bash(SandboxCommandRequest {
                command: "echo no-host".into(),
                working_dir: std::path::PathBuf::from("."),
                timeout_ms: 1000,
                max_output_bytes: 1024,
                env: Vec::new(),
            })
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("no host shell fallback"));
    }
}
