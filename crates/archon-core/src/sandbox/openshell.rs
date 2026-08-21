use std::future::Future;
use std::pin::Pin;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxCommandRequest, SandboxCommandResult, SandboxScope, SandboxScopeSupport,
    SandboxTerminal, SandboxTerminalRequest,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

mod exec;
mod fs;

use exec::{openshell_create_args, openshell_output_result};
pub use fs::openshell_filesystem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenShellConfig {
    pub enabled: bool,
    pub binary: String,
    pub workspace_mode: String,
    pub gateway: Option<String>,
    pub remote_workdir: Option<String>,
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
            workspace_mode: "upload".into(),
            gateway: Some("openshell".into()),
            remote_workdir: None,
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
        if let Some(gateway) = self.gateway.as_deref()
            && gateway.contains('\0')
        {
            return Err("sandbox.openshell.gateway must not contain NUL".into());
        }
        if let Some(workdir) = self.remote_workdir.as_deref()
            && workdir.contains('\0')
        {
            return Err("sandbox.openshell.remote_workdir must not contain NUL".into());
        }
        if let Some(policy) = self.policy.as_deref()
            && policy.contains('\0')
        {
            return Err("sandbox.openshell.policy must not contain NUL".into());
        }
        match self.workspace_mode.as_str() {
            "mirror" | "remote" | "upload" => Ok(()),
            other => Err(format!(
                "sandbox.openshell.workspace_mode must be mirror, remote, or upload, got \"{other}\""
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
    } else if matches!(config.workspace_mode.as_str(), "remote" | "upload")
        && config.gateway.as_deref().unwrap_or("").is_empty()
    {
        findings.push("remote/upload workspace mode requires an explicit gateway".into());
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

/// Why this backend has no terminal to offer (#201 Phase 6).
///
/// Every command here is `openshell sandbox create --no-keep -- ...`: a sandbox
/// built for one command and destroyed with it, and in `upload` mode a fresh
/// copy of the workspace each time. There is no session for a PTY to attach to,
/// and a sandbox held open for a terminal would be a *different* world from the
/// one the next `Bash` call creates — so the shell and the commands around it
/// would disagree about what is on disk. Refusing says that; opening one on the
/// host would hide it.
const NO_PERSISTENT_SESSION: &str = "the openshell backend creates a throwaway \
     sandbox per command, so there is no session for a terminal to live in. Use \
     Bash, or switch sandbox.backend to docker for a shell inside the sandbox";

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
        if matches!(self.config.workspace_mode.as_str(), "remote" | "upload")
            && self
                .config
                .gateway
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err("openshell remote/upload mode requires an explicit gateway".into());
        }
        Ok(())
    }
}

impl SandboxBackend for OpenShellSandboxBackend {
    fn check(
        &self,
        tool: &str,
        capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        self.safe_to_route()?;
        crate::sandbox::capability_gate::check_capability("openshell", tool, capability).map(|_| ())
    }

    fn terminal(&self, _request: &SandboxTerminalRequest) -> SandboxTerminal {
        SandboxTerminal::Refused(format!("openshell sandbox: {NO_PERSISTENT_SESSION}"))
    }

    /// `tool` is the only lifetime this backend can honestly claim.
    ///
    /// Every command is `openshell sandbox create --no-keep --`, which builds a
    /// sandbox and destroys it on exit; in `upload` mode the working directory
    /// is re-uploaded each time as well. Nothing survives, so `session` and
    /// `turn` would be claims the backend cannot keep, and this refuses them at
    /// config load rather than pretending.
    ///
    /// Not a permanent verdict, and deliberately not guessed at: whether the
    /// OpenShell CLI can exec into an existing sandbox was not checkable here
    /// because the binary is not installed on this machine. What would need
    /// establishing is whether `sandbox create` without `--no-keep` yields a
    /// durable handle and whether some `sandbox exec`/`attach` verb can run a
    /// command in it; if both hold, this becomes `Held` for `session` and
    /// `turn` and `terminal` stops having to refuse.
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        match scope {
            SandboxScope::Tool => SandboxScopeSupport::PerCommand,
            SandboxScope::Session | SandboxScope::Turn => {
                SandboxScopeSupport::Unsupported(format!(
                    "sandbox.scope = \"{scope}\" asks for a sandbox that outlives one command, \
                     and the openshell backend runs `sandbox create --no-keep` per command, so \
                     nothing survives it. Set sandbox.scope = \"tool\", which is what this \
                     backend actually does, or switch sandbox.backend to docker"
                ))
            }
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
                exit_code: None,
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
                    exit_code: None,
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
                    exit_code: None,
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
                exit_code: None,
            },
            Err(_) => SandboxCommandResult {
                content: format!(
                    "Error: OpenShell command timed out after {}ms",
                    request.timeout_ms
                ),
                is_error: true,
                exit_code: None,
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
    if let Some(gateway) = config.gateway.as_deref().map(str::trim)
        && !gateway.is_empty()
    {
        cmd.env("OPENSHELL_GATEWAY", gateway);
    }
}

#[cfg(test)]
#[path = "openshell/tests.rs"]
mod tests;
