use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxCommandRequest, SandboxCommandResult, SandboxScope, SandboxScopeSupport,
    SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

mod exec;
mod fs;
mod pool;
mod reap;

pub use fs::DockerFs;

use exec::{
    container_shell, container_workdir, docker_output_result, docker_run_args,
    docker_terminal_args, normal_writable_path, validate_workspace_access,
};
use pool::{ContainerPool, DEFAULT_MAX_AGE_SECS, max_age_is_sane};

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
    /// Hard upper bound on how long a held container lives (`sandbox.scope` of
    /// `session` or `turn`).
    ///
    /// It is the container's own `sleep`, not a host-side timer, so it holds
    /// even when Archon is SIGKILLed and never restarted. A command still
    /// running when it expires is killed with the container and the next
    /// command rebuilds it, so this must stay well above any Bash timeout.
    pub container_max_age_secs: u64,
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
            container_max_age_secs: DEFAULT_MAX_AGE_SECS,
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
        }?;
        for path in &self.writable_paths {
            normal_writable_path(path)?;
        }
        max_age_is_sane(self.container_max_age_secs)
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

#[derive(Debug, Clone)]
pub struct DockerSandboxBackend {
    config: DockerConfig,
    workspace_access: String,
    /// Shared across clones on purpose. The backend is handed out as an `Arc<dyn
    /// SandboxBackend>` but is `Clone`, and a pool copied per clone would hold
    /// one container per copy while every copy reported reuse.
    pool: Arc<ContainerPool>,
}

impl DockerSandboxBackend {
    pub fn new(
        config: DockerConfig,
        workspace_access: impl Into<String>,
        scope: SandboxScope,
    ) -> Self {
        let workspace_access = workspace_access.into();
        Self {
            pool: Arc::new(ContainerPool::new(
                config.clone(),
                workspace_access.clone(),
                scope,
            )),
            config,
            workspace_access,
        }
    }

    fn safe_to_execute(&self) -> Result<(), String> {
        self.config.validate()?;
        validate_workspace_access(&self.workspace_access)?;
        if !self.config.enabled {
            return Err("docker sandbox backend is disabled".into());
        }
        if self.config.privileged {
            return Err("docker sandbox refuses privileged containers".into());
        }
        if self.config.mount_docker_socket {
            return Err("docker sandbox refuses host Docker socket mounts".into());
        }
        if self.config.mount_home {
            return Err("docker sandbox refuses broad host home mounts".into());
        }
        Ok(())
    }
}

impl SandboxBackend for DockerSandboxBackend {
    fn check(
        &self,
        tool: &str,
        capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        self.safe_to_execute()?;
        crate::sandbox::capability_gate::check_capability("docker", tool, capability).map(|_| ())
    }

    /// A terminal is a container with a TTY on it.
    ///
    /// The bind mount makes this the same world `execute_bash` runs in, down to
    /// the bytes, so a shell opened here sees what `Bash` sees and writes what
    /// `Read` reads.
    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        if let Err(error) = self.safe_to_execute() {
            return SandboxTerminal::Refused(format!("docker sandbox: {error}"));
        }
        let (shell, program) = match container_shell(request.shell.as_deref()) {
            Ok(resolved) => resolved,
            Err(error) => return SandboxTerminal::Refused(error),
        };
        let workdir = match container_workdir(&request.workspace, &request.cwd) {
            Ok(workdir) => workdir,
            Err(error) => return SandboxTerminal::Refused(format!("docker sandbox: {error}")),
        };
        SandboxTerminal::Open(SandboxTerminalCommand {
            program: self.config.binary.clone(),
            args: docker_terminal_args(
                &self.config,
                &self.workspace_access,
                &request.workspace,
                &workdir,
                &program,
            ),
            shell,
            location: format!("{workdir} in the {} container", self.config.image),
        })
    }

    /// Docker can hold a container open for any of the three lifetimes.
    ///
    /// `tool` is not a degraded answer here: it is what the backend did for
    /// every scope before this existed, and an operator who wants a container
    /// per command can still have one.
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        match scope {
            SandboxScope::Tool => SandboxScopeSupport::PerCommand,
            SandboxScope::Session | SandboxScope::Turn => SandboxScopeSupport::Held,
        }
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async move { Some(self.execute_bash_inner(request).await) })
    }
}

impl DockerSandboxBackend {
    async fn execute_bash_inner(&self, request: SandboxCommandRequest) -> SandboxCommandResult {
        if let Err(error) = self.safe_to_execute() {
            return SandboxCommandResult {
                content: format!("Error: {error}"),
                is_error: true,
                exit_code: None,
            };
        }
        match self.pool.container_for(&request).await {
            Ok(Some(name)) => self.execute_in_held(&request, name).await,
            // No lifetime is held for this request — `tool` scope, or `turn`
            // scope from a caller with no turn identity. Both mean one container
            // per command, which is what this path has always done.
            Ok(None) => self.execute_in_fresh(&request).await,
            Err(error) => SandboxCommandResult {
                content: format!("Error: {error}"),
                is_error: true,
                exit_code: None,
            },
        }
    }

    /// One command in the held container, rebuilt once if it has gone away.
    ///
    /// The retry is bounded to a single attempt and only taken when the daemon
    /// itself confirms the container is not running, so a command that fails on
    /// its own merits is never silently run twice.
    async fn execute_in_held(
        &self,
        request: &SandboxCommandRequest,
        name: String,
    ) -> SandboxCommandResult {
        let first = self
            .spawn_docker(self.pool.exec_args(&name, request), request)
            .await;
        if !looks_like_a_missing_container(&first)
            || !self.pool.forget_if_gone(request, &name).await
        {
            return first;
        }
        match self.pool.container_for(request).await {
            Ok(Some(rebuilt)) => {
                self.spawn_docker(self.pool.exec_args(&rebuilt, request), request)
                    .await
            }
            Ok(None) => self.execute_in_fresh(request).await,
            Err(error) => SandboxCommandResult {
                content: format!("Error: {error}"),
                is_error: true,
                exit_code: None,
            },
        }
    }

    async fn execute_in_fresh(&self, request: &SandboxCommandRequest) -> SandboxCommandResult {
        let args = docker_run_args(&self.config, &self.workspace_access, request);
        self.spawn_docker(args, request).await
    }

    async fn spawn_docker(
        &self,
        args: Vec<String>,
        request: &SandboxCommandRequest,
    ) -> SandboxCommandResult {
        let mut cmd = TokioCommand::new(&self.config.binary);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                return SandboxCommandResult {
                    content: format!("Error: Failed to spawn docker: {error}"),
                    is_error: true,
                    exit_code: None,
                };
            }
        };

        let result = tokio::time::timeout(Duration::from_millis(request.timeout_ms), async {
            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();
            if let Some(mut stdout) = child.stdout.take() {
                let _ = stdout.read_to_end(&mut stdout_buf).await;
            }
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_end(&mut stderr_buf).await;
            }
            let status = child.wait().await;
            (stdout_buf, stderr_buf, status)
        })
        .await;

        match result {
            Ok((stdout_buf, stderr_buf, status)) => {
                docker_output_result(stdout_buf, stderr_buf, status, request.max_output_bytes)
            }
            Err(_) => {
                // Kills the local docker client, not the process it started
                // inside the container. Under a held container that process
                // survives to the end of the scope, against that container's
                // pids and memory limits; `container_max_age_secs` is what
                // eventually collects it. Tracking exec ids to kill it properly
                // is not done.
                let _ = child.kill().await;
                SandboxCommandResult {
                    content: format!(
                        "Error: Docker command timed out after {}ms",
                        request.timeout_ms
                    ),
                    is_error: true,
                    exit_code: None,
                }
            }
        }
    }
}

/// A hint that a held container has gone away — never the decision.
///
/// Docker reports a vanished container as exit 1 with the daemon's own prefix,
/// which a user command is free to print too. It is used only to decide whether
/// to *ask* the daemon, which then answers authoritatively in
/// `ContainerPool::forget_if_gone`. A false positive costs one `docker inspect`
/// and changes nothing else; a false negative surfaces the daemon's error to the
/// model, which is the truth.
fn looks_like_a_missing_container(result: &SandboxCommandResult) -> bool {
    result.is_error && result.content.contains("Error response from daemon:")
}

#[cfg(test)]
#[path = "docker/tests.rs"]
mod tests;
