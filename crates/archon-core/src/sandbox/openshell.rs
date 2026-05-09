use std::future::Future;
use std::pin::Pin;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

mod exec;

use exec::{openshell_create_args, openshell_output_result};

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
        if self.binary.contains('\0') {
            return Err("sandbox.openshell.binary must not contain NUL".into());
        }
        if let Some(gateway) = self.gateway.as_deref() {
            if gateway.contains('\0') {
                return Err("sandbox.openshell.gateway must not contain NUL".into());
            }
        }
        if let Some(policy) = self.policy.as_deref() {
            if policy.contains('\0') {
                return Err("sandbox.openshell.policy must not contain NUL".into());
            }
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
    findings.push(
        "doctor is detect-only; Bash execution routes through OpenShell when selected".into(),
    );
    findings.push(
        "provider injection is disabled by default; Anthropic spoofing remains host-side".into(),
    );
    if !config.providers.is_empty() && !config.provider_injection {
        findings.push(
            "configured OpenShell providers are ignored while provider_injection=false".into(),
        );
    }

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
    out.push_str("Execution: Bash routes through OpenShell when sandbox.backend=openshell\n");
    out
}

#[derive(Debug, Clone)]
pub struct OpenShellSandboxBackend {
    config: OpenShellConfig,
}

impl OpenShellSandboxBackend {
    pub fn new(config: OpenShellConfig) -> Self {
        Self { config }
    }

    fn safe_to_route(&self) -> Result<(), String> {
        self.config.validate()?;
        if self.config.provider_injection {
            return Err("openshell sandbox refuses provider injection by default".into());
        }
        if self.config.host_shell_fallback {
            return Err("openshell sandbox refuses host shell fallback".into());
        }
        if !self.config.enabled {
            return Err("openshell sandbox backend is disabled".into());
        }
        let probe = probe_openshell(&self.config.binary);
        if !probe.found {
            return Err(probe
                .error
                .unwrap_or_else(|| "openshell binary was not found".into()));
        }
        if self.config.workspace_mode == "remote"
            && self
                .config
                .gateway
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("openshell remote mode requires an explicit gateway".into());
        }
        Ok(())
    }
}

impl SandboxBackend for OpenShellSandboxBackend {
    fn check(&self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
        self.safe_to_route()?;
        match tool {
            "Read" | "Glob" | "Grep" | "ToolSearch" | "TodoWrite" | "Sleep" => Ok(()),
            "Bash" | "Shell" => Ok(()),
            "Write" | "Edit" | "NotebookEdit" => Err(format!(
                "openshell sandbox: {tool} host-side file mutation is not supported"
            )),
            "WebFetch" | "WebSearch" => Err(format!(
                "openshell sandbox: {tool} host-side network access is not supported"
            )),
            "TaskCreate" | "TaskUpdate" | "Agent" => Err(format!(
                "openshell sandbox: {tool} agent spawning is not supported"
            )),
            other => Err(format!("openshell sandbox: unsupported tool {other}")),
        }
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async move { Some(self.execute_bash_inner(request).await) })
    }
}

impl OpenShellSandboxBackend {
    async fn execute_bash_inner(&self, request: SandboxCommandRequest) -> SandboxCommandResult {
        if let Err(error) = self.safe_to_route() {
            return SandboxCommandResult {
                content: format!(
                    "OpenShell sandbox refused execution: {error}; no host shell fallback was used.\n"
                ),
                is_error: true,
            };
        }
        let args = match openshell_create_args(&self.config, &request) {
            Ok(args) => args,
            Err(error) => {
                return SandboxCommandResult {
                    content: format!(
                        "OpenShell sandbox refused execution: {error}; no host shell fallback was used.\n"
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
        apply_openshell_env_policy(&mut cmd, &self.config);
        #[cfg(unix)]
        cmd.process_group(0);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                return SandboxCommandResult {
                    content: format!("Error: Failed to spawn openshell: {error}"),
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
            Ok(Ok(output)) => openshell_output_result(output, request.max_output_bytes),
            Ok(Err(error)) => SandboxCommandResult {
                content: format!("Error: OpenShell command failed: {error}"),
                is_error: true,
            },
            Err(_) => SandboxCommandResult {
                content: format!(
                    "Error: OpenShell command timed out after {}ms",
                    request.timeout_ms
                ),
                is_error: true,
            },
        }
    }
}

fn apply_openshell_env_policy(cmd: &mut TokioCommand, config: &OpenShellConfig) {
    for name in [
        "ANTHROPIC_API_KEY",
        "CLAUDE_API_KEY",
        "OPENAI_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GITLAB_TOKEN",
        "NVIDIA_API_KEY",
        "COPILOT_GITHUB_TOKEN",
    ] {
        cmd.env_remove(name);
    }
    if let Some(gateway) = config.gateway.as_deref().map(str::trim) {
        if !gateway.is_empty() {
            cmd.env("OPENSHELL_GATEWAY", gateway);
        }
    }
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

    #[test]
    fn doctor_reports_routed_execution_without_provider_injection() {
        let cfg = OpenShellConfig {
            enabled: true,
            providers: vec!["my-claude".into()],
            ..OpenShellConfig::default()
        };

        let report = openshell_doctor_report(&cfg, OpenShellProbe::found("openshell 1.2.3"));
        let body = render_openshell_doctor_report(&report);

        assert_eq!(report.status, OpenShellDoctorStatus::ReadyDetectOnly);
        assert!(body.contains("Bash routes through OpenShell"));
        assert!(body.contains("providers are ignored"));
        assert!(body.contains("Anthropic spoofing remains host-side"));
    }

    #[test]
    fn backend_fails_closed_when_openshell_missing() {
        let backend = OpenShellSandboxBackend::new(OpenShellConfig {
            enabled: true,
            binary: "__definitely_missing_openshell__".into(),
            ..OpenShellConfig::default()
        });

        let error = backend.check("Bash", &serde_json::json!({})).unwrap_err();

        assert!(error.contains("__definitely_missing_openshell__"));
    }

    #[tokio::test]
    async fn backend_execute_bash_runs_fail_closed_preflight() {
        let backend = OpenShellSandboxBackend::new(OpenShellConfig {
            enabled: true,
            binary: "__definitely_missing_openshell__".into(),
            ..OpenShellConfig::default()
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
        assert!(result.content.contains("refused execution"));
        assert!(result.content.contains("__definitely_missing_openshell__"));
        assert!(result.content.contains("no host shell fallback"));
    }
}
