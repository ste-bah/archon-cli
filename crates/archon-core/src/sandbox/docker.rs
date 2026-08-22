use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxCommandRequest, SandboxCommandResult, SandboxScope, SandboxScopeSupport,
    SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

mod doctor;
mod exec;
mod fs;
mod pool;
mod reap;

pub use doctor::{
    DockerDoctorReport, DockerDoctorStatus, DockerProbe, docker_doctor_report, probe_docker,
    render_docker_doctor_report,
};
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
            Ok(Some(lease)) => self.execute_in_held(&request, lease).await,
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
    /// Two different failures, treated differently on purpose. The daemon
    /// refusing the `exec` means the command never started, so rebuilding and
    /// running it once is safe. A command *killed* mid-flight already ran, so it
    /// is never re-run — re-running would repeat whatever side effects it had
    /// got through. It is annotated instead, because a bare `Exit code 137` for
    /// a container that disappeared underneath the command tells the model
    /// nothing it can act on.
    ///
    /// Both paths ask the daemon before concluding anything, so a command that
    /// failed on its own merits is neither retried nor annotated.
    async fn execute_in_held(
        &self,
        request: &SandboxCommandRequest,
        lease: pool::ContainerLease,
    ) -> SandboxCommandResult {
        let first = self
            .spawn_docker(self.pool.exec_args(lease.name(), request), request)
            .await;
        if !first.is_error {
            return first;
        }
        if looks_like_a_missing_container(&first) {
            if !self.pool.forget_if_gone(request, lease.name()).await {
                return first;
            }
            // Released before the rebuild so the vanished container's count
            // cannot keep a replacement's eviction waiting on it.
            drop(lease);
            return match self.pool.container_for(request).await {
                Ok(Some(rebuilt)) => {
                    self.spawn_docker(self.pool.exec_args(rebuilt.name(), request), request)
                        .await
                }
                Ok(None) => self.execute_in_fresh(request).await,
                Err(error) => SandboxCommandResult {
                    content: format!("Error: {error}"),
                    is_error: true,
                    exit_code: None,
                },
            };
        }
        if looks_like_the_container_died_under_the_command(&first)
            && self.pool.forget_if_gone(request, lease.name()).await
        {
            return annotate_lost_container(first);
        }
        first
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

/// A hint that the container was destroyed while the command was inside it.
///
/// 137 is 128+SIGKILL, which is what `docker exec` reports when the container it
/// was running in is force-removed. An ordinary command can exit 137 too — it is
/// what a host OOM kill looks like — so this only decides whether to *ask* the
/// daemon whether the container is still there.
fn looks_like_the_container_died_under_the_command(result: &SandboxCommandResult) -> bool {
    matches!(result.exit_code, Some(137) | None)
}

/// Say what happened, because `Exit code 137` on its own reads as a memory limit
/// and sends the model looking in the wrong place.
fn annotate_lost_container(mut result: SandboxCommandResult) -> SandboxCommandResult {
    result.content.push_str(
        "\n\nThe sandbox container this command was running in stopped before the \
         command finished, so the command was killed rather than failing on its \
         own. The next command will start a fresh container. If this repeats, \
         sandbox.docker.container_max_age_secs may be shorter than the commands \
         being run.",
    );
    result
}

#[cfg(test)]
#[path = "docker/tests.rs"]
mod tests;
