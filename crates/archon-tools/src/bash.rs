use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::provider_env::{ProviderEnvPolicy, ProviderEnvResolution, ProviderEnvSource};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const SENSITIVE_PATTERNS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "_TOKEN",
    "_SECRET",
    "_KEY",
    "_PASSWORD",
    "_CREDENTIAL",
];

const PASSTHROUGH_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "EDITOR",
    "VISUAL",
    "TMPDIR",
    "TMP",
    "TEMP",
];

const DEFAULT_BASH_TIMEOUT_SECS: u64 = 86_400;
const BASH_COMPAT_PRELUDE: &str = r#"
printf() {
    if [ "${1-}" = "-v" ]; then
        builtin printf "$@"
        return
    fi
    if [ "${1-}" = "--" ]; then
        shift
        builtin printf -- "$@"
        return
    fi
    builtin printf -- "$@"
}
"#;

const SHELL_TIMEOUT_PRELUDE: &str = r#"
timeout() {
    while [ "$#" -gt 0 ]; do
        case "${1-}" in
            --) shift; break ;;
            -k|--kill-after|-s|--signal)
                shift
                if [ "$#" -gt 0 ]; then shift; fi
                ;;
            --foreground|--preserve-status|-v|--verbose)
                shift
                ;;
            -*)
                shift
                ;;
            *[0-9]s|*[0-9]m|*[0-9]h|*[0-9]d|[0-9]*|[0-9]*.*)
                shift
                break
                ;;
            *)
                break
                ;;
        esac
    done
    if [ "$#" -eq 0 ]; then
        return 125
    fi
    "$@"
}

gtimeout() {
    timeout "$@"
}
"#;

pub struct BashTool {
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    pub provider_env: Option<ProviderEnvSource>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_BASH_TIMEOUT_SECS,
            max_output_bytes: 102400,
            provider_env: None,
        }
    }
}

impl BashTool {
    pub fn with_provider_env(mut self, provider_env: ProviderEnvPolicy) -> Self {
        self.provider_env = Some(ProviderEnvSource::Policy(provider_env));
        self
    }

    pub fn with_provider_env_resolution(mut self, provider_env: ProviderEnvResolution) -> Self {
        self.provider_env = Some(ProviderEnvSource::Resolution(provider_env));
        self
    }

    pub fn with_provider_env_source(mut self, provider_env: ProviderEnvSource) -> Self {
        self.provider_env = Some(provider_env);
        self
    }
}

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Executes a bash command and returns its output."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds. Leave unset unless the user or task explicitly requests a longer per-command timeout; shorter values do not undercut the configured tools.bash_timeout."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let raw_command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("command is required and must be a string"),
        };
        let timeout_ms = effective_timeout_ms(
            input.get("timeout").and_then(|v| v.as_u64()),
            self.timeout_secs * 1000,
        );

        let mut env_vars = sanitized_env();
        ensure_env_default(&mut env_vars, "CARGO_INCREMENTAL", "0");
        crate::workflow_resource_env::apply_workflow_resource_defaults(&mut env_vars, raw_command);
        let provider_env = provider_env_overlay(self.provider_env.as_ref()).await;
        if let Some(provider_env) = &provider_env {
            provider_env.apply_to_env(&mut env_vars);
        }
        let _cargo_target_lock = match crate::cargo_target_env::apply_cargo_target_dir_guard(
            &mut env_vars,
            raw_command,
            &ctx.working_dir,
            &ctx.session_id,
            ctx.cancel_parent.clone(),
        )
        .await
        {
            Ok(lock) => lock,
            Err(message) => return ToolResult::error(message),
        };
        let guarded_command = crate::cargo_target_env::enforce_host_cargo_target_dir(
            raw_command,
            _cargo_target_lock.is_some(),
        );
        let repair_prelude =
            crate::cargo_target_env::cargo_cache_repair_prelude(_cargo_target_lock.as_ref());
        let guarded_command = if repair_prelude.is_empty() {
            guarded_command
        } else {
            format!("{repair_prelude}\n{guarded_command}")
        };
        let command = command_with_compat_prelude(&guarded_command);

        if let Some(sandbox) = &ctx.sandbox
            && let Some(result) = sandbox
                .execute_bash(archon_permissions::sandbox::SandboxCommandRequest {
                    command: command.clone(),
                    working_dir: ctx.working_dir.clone(),
                    timeout_ms,
                    max_output_bytes: self.max_output_bytes,
                    env: env_vars.clone(),
                })
                .await
        {
            return ToolResult {
                content: redact_provider_env_output(provider_env.as_ref(), result.content),
                is_error: result.is_error,
            };
        }

        let mut cmd = Command::new("/bin/bash");
        cmd.arg("-c")
            .arg(&command)
            .current_dir(&ctx.working_dir)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        #[cfg(unix)]
        cmd.process_group(0); // new process group for clean kill
        cmd.kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to spawn bash: {e}")),
        };

        // Read output with timeout AND respect parent cancellation token.
        // Bug-fix 2026-05-12: previously this only enforced the timeout; the
        // CancellationToken from `ctx.cancel_parent` was ignored, so Ctrl+C /
        // double-Esc could not interrupt a long-running Bash command spawned
        // by a subagent. We now race three signals — completion, timeout,
        // cancel — and kill the process group on either non-completion path.
        let timeout_dur = Duration::from_millis(timeout_ms);
        // Fall back to a fresh (never-cancelled) token when there's no parent
        // chain, so the `select!` arm shape stays uniform.
        let cancel_token = ctx.cancel_parent.clone().unwrap_or_default();

        let stdout_bytes = Arc::new(AtomicUsize::new(0));
        let stderr_bytes = Arc::new(AtomicUsize::new(0));
        let heartbeat = crate::bash_observability::start_bash_heartbeat(
            ctx,
            child.id(),
            timeout_ms,
            raw_command,
            stdout_bytes.clone(),
            stderr_bytes.clone(),
        );
        let stdout_task = spawn_pipe_reader(child.stdout.take(), stdout_bytes);
        let stderr_task = spawn_pipe_reader(child.stderr.take(), stderr_bytes);

        enum BashOutcome {
            Done(std::io::Result<std::process::ExitStatus>),
            Timeout,
            Cancelled,
        }

        let work = tokio::time::timeout(timeout_dur, child.wait());

        let outcome = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => BashOutcome::Cancelled,
            res = work => match res {
                Ok(status) => BashOutcome::Done(status),
                Err(_) => BashOutcome::Timeout,
            }
        };
        crate::bash_observability::stop_bash_heartbeat(heartbeat);

        match outcome {
            BashOutcome::Done(status) => {
                let (stdout_buf, stderr_buf) = join_output(stdout_task, stderr_task).await;
                let exit_code = status.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);

                let mut output = String::new();

                // Combine stdout and stderr
                let combined = [stdout_buf, stderr_buf].concat();
                let truncated = combined.len() > self.max_output_bytes;
                let bytes = if truncated {
                    &combined[..self.max_output_bytes]
                } else {
                    &combined
                };

                output.push_str(&String::from_utf8_lossy(bytes));

                if truncated {
                    output.push_str(&format!(
                        "\n\nOutput truncated at {} bytes",
                        self.max_output_bytes
                    ));
                }

                if exit_code != 0 {
                    ToolResult {
                        content: redact_provider_env_output(
                            provider_env.as_ref(),
                            format!("Exit code {exit_code}\n{output}"),
                        ),
                        is_error: true,
                    }
                } else {
                    ToolResult::success(redact_provider_env_output(provider_env.as_ref(), output))
                }
            }
            BashOutcome::Timeout => {
                terminate_child(&mut child, "timeout").await;
                let _ = join_output(stdout_task, stderr_task).await;
                ToolResult::error(format!("Command timed out after {}ms", timeout_ms))
            }
            BashOutcome::Cancelled => {
                terminate_child(&mut child, "parent cancellation").await;
                let _ = join_output(stdout_task, stderr_task).await;
                tracing::info!("bash: command cancelled by parent CancellationToken");
                ToolResult::error("Command cancelled by user".to_string())
            }
        }
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

        match archon_permissions::classifier::classify_command(command, &[], &[], &[]) {
            archon_permissions::classifier::CommandClass::Safe => PermissionLevel::Safe,
            archon_permissions::classifier::CommandClass::Risky => PermissionLevel::Risky,
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
        }
    }
}

async fn provider_env_overlay(source: Option<&ProviderEnvSource>) -> Option<ProviderEnvResolution> {
    let source = source?;
    if let Some(resolved) = source.resolution()
        && source.policy().is_none_or(|policy| resolved.covers(policy))
    {
        return Some(resolved.clone());
    }
    let policy = source.policy()?;
    Some(crate::provider_env::resolve_provider_env(policy).await)
}

fn redact_provider_env_output(
    provider_env: Option<&ProviderEnvResolution>,
    output: String,
) -> String {
    provider_env.map_or(output.clone(), |env| env.redact_text(&output))
}

fn effective_timeout_ms(requested_ms: Option<u64>, configured_ms: u64) -> u64 {
    let Some(requested_ms) = requested_ms else {
        return configured_ms;
    };
    requested_ms.max(configured_ms)
}

fn spawn_pipe_reader<T>(pipe: Option<T>, byte_count: Arc<AtomicUsize>) -> JoinHandle<Vec<u8>>
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut chunk = [0_u8; 8192];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(read) => {
                        byte_count.fetch_add(read, Ordering::Relaxed);
                        buffer.extend_from_slice(&chunk[..read]);
                    }
                    Err(_) => break,
                }
            }
        }
        buffer
    })
}

async fn join_output(
    stdout_task: JoinHandle<Vec<u8>>,
    stderr_task: JoinHandle<Vec<u8>>,
) -> (Vec<u8>, Vec<u8>) {
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    (stdout, stderr)
}

async fn terminate_child(child: &mut Child, reason: &str) {
    let pid = child.id();
    #[cfg(unix)]
    if let Some(pid) = pid {
        signal_process_group(pid, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill().await;
    }

    let exited = tokio::time::timeout(Duration::from_millis(500), child.wait())
        .await
        .is_ok();

    #[cfg(unix)]
    if let Some(pid) = pid {
        signal_process_group(pid, libc::SIGKILL);
    }
    if exited {
        return;
    }

    let _ = child.kill().await;
    let _ = child.wait().await;
    tracing::info!(reason, "bash: terminated process group");
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: libc::c_int) {
    let pgid = -(pid as libc::pid_t);
    // SAFETY: `kill` is called with a process-group id derived from the child
    // pid returned by std/tokio after a successful spawn.
    unsafe {
        libc::kill(pgid, signal);
    }
}

fn command_with_compat_prelude(command: &str) -> String {
    format!("{BASH_COMPAT_PRELUDE}\n{SHELL_TIMEOUT_PRELUDE}\n{command}")
}

pub fn sanitized_env() -> Vec<(String, String)> {
    let mut env = Vec::new();

    for (key, value) in std::env::vars() {
        if PASSTHROUGH_VARS.contains(&key.as_str()) {
            env.push((key, value));
            continue;
        }

        let upper = key.to_uppercase();
        let is_sensitive = SENSITIVE_PATTERNS
            .iter()
            .any(|pattern| upper.contains(pattern));

        if !is_sensitive {
            env.push((key, value));
        }
    }

    env
}

fn ensure_env_default(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !env.iter().any(|(existing, _)| existing == key) {
        env.push((key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
