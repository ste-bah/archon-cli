use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Sensitive environment variable patterns to strip before spawning.
const SENSITIVE_PATTERNS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "_TOKEN",
    "_SECRET",
    "_KEY",
    "_PASSWORD",
    "_CREDENTIAL",
];

/// Environment variables to always pass through.
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

const DEFAULT_BASH_TIMEOUT_SECS: u64 = 600;
const PIPE_READ_CHUNK_BYTES: usize = 8 * 1024;

static BASH_PROGRAM: LazyLock<PathBuf> =
    LazyLock::new(|| select_bash_program(which::which("bash").ok(), which::which("bash.exe").ok()));

fn select_bash_program(bash: Option<PathBuf>, bash_exe: Option<PathBuf>) -> PathBuf {
    bash.or(bash_exe).unwrap_or_else(|| PathBuf::from("bash"))
}

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
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_BASH_TIMEOUT_SECS,
            max_output_bytes: 102400,
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
        }
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
                    "description": "Optional timeout in milliseconds. Values below the configured tools.bash_timeout shorten this command; larger values are clamped to that configured maximum."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("command is required and must be a string"),
        };
        let command = command_with_compat_prelude(command);

        let timeout_ms = effective_timeout_ms(
            input.get("timeout").and_then(|v| v.as_u64()),
            self.timeout_secs * 1000,
        );

        // Build sanitized environment
        let mut env_vars = sanitized_env();
        ensure_env_default(&mut env_vars, "CARGO_INCREMENTAL", "0");

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
                content: result.content,
                is_error: result.is_error,
            };
        }

        let mut cmd = Command::new(BASH_PROGRAM.as_path());
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

        let remaining_output = Arc::new(AtomicUsize::new(self.max_output_bytes));
        let stdout_task = spawn_pipe_reader(child.stdout.take(), Arc::clone(&remaining_output));
        let stderr_task = spawn_pipe_reader(child.stderr.take(), remaining_output);

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

        match outcome {
            BashOutcome::Done(status) => {
                let (stdout_capture, stderr_capture) = join_output(stdout_task, stderr_task).await;
                let exit_code = status.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);

                let combined = [stdout_capture.bytes, stderr_capture.bytes].concat();
                let truncated = stdout_capture.truncated || stderr_capture.truncated;
                let mut output = String::from_utf8_lossy(&combined).into_owned();

                if truncated {
                    output.push_str(&format!(
                        "\n\nOutput truncated at {} bytes",
                        self.max_output_bytes
                    ));
                }

                if exit_code != 0 {
                    ToolResult {
                        content: format!("Exit code {exit_code}\n{output}"),
                        is_error: true,
                    }
                } else {
                    ToolResult::success(output)
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

        match archon_permissions::classifier::classify_command(
            command,
            &self.safe_commands,
            &self.risky_commands,
            &self.dangerous_commands,
        ) {
            archon_permissions::classifier::CommandClass::Safe => PermissionLevel::Safe,
            archon_permissions::classifier::CommandClass::Risky => PermissionLevel::Risky,
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
        }
    }
}

fn effective_timeout_ms(requested_ms: Option<u64>, configured_ms: u64) -> u64 {
    requested_ms.unwrap_or(configured_ms).min(configured_ms)
}

struct CapturedPipe {
    bytes: Vec<u8>,
    truncated: bool,
}

fn spawn_pipe_reader<T>(pipe: Option<T>, remaining: Arc<AtomicUsize>) -> JoinHandle<CapturedPipe>
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut truncated = false;
        if let Some(mut pipe) = pipe {
            let mut chunk = [0_u8; PIPE_READ_CHUNK_BYTES];
            loop {
                let read = match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let retained = reserve_output_bytes(&remaining, read);
                bytes.extend_from_slice(&chunk[..retained]);
                truncated |= retained < read;
            }
        }
        CapturedPipe { bytes, truncated }
    })
}

fn reserve_output_bytes(remaining: &AtomicUsize, requested: usize) -> usize {
    let mut available = remaining.load(Ordering::Relaxed);
    loop {
        let retained = available.min(requested);
        match remaining.compare_exchange_weak(
            available,
            available - retained,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return retained,
            Err(current) => available = current,
        }
    }
}

fn empty_capture() -> CapturedPipe {
    CapturedPipe {
        bytes: Vec::new(),
        truncated: false,
    }
}

async fn join_output(
    stdout_task: JoinHandle<CapturedPipe>,
    stderr_task: JoinHandle<CapturedPipe>,
) -> (CapturedPipe, CapturedPipe) {
    let stdout = stdout_task.await.unwrap_or_else(|_| empty_capture());
    let stderr = stderr_task.await.unwrap_or_else(|_| empty_capture());
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

    if tokio::time::timeout(Duration::from_millis(500), child.wait())
        .await
        .is_ok()
    {
        return;
    }

    #[cfg(unix)]
    if let Some(pid) = pid {
        signal_process_group(pid, libc::SIGKILL);
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

/// Build a sanitized environment map.
/// Public so PowerShell tool can reuse the same sanitization.
pub fn sanitized_env() -> Vec<(String, String)> {
    let mut env = Vec::new();

    for (key, value) in std::env::vars() {
        // Check if this is a passthrough var
        if PASSTHROUGH_VARS.contains(&key.as_str()) {
            env.push((key, value));
            continue;
        }

        // Check if this matches a sensitive pattern
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
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("."),
            ..ToolContext::default()
        }
    }

    #[test]
    fn bash_program_selection_prefers_path_discovery() {
        let bash = PathBuf::from("/usr/local/bin/bash");
        let bash_exe = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        assert_eq!(
            select_bash_program(Some(bash.clone()), Some(bash_exe.clone())),
            bash
        );
        assert_eq!(select_bash_program(None, Some(bash_exe.clone())), bash_exe);
        assert_eq!(select_bash_program(None, None), PathBuf::from("bash"));
    }

    #[tokio::test]
    async fn pipe_reader_caps_storage_and_drains_remaining_bytes() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let task = spawn_pipe_reader(Some(reader), Arc::new(AtomicUsize::new(5)));
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"abcdefghij").await.unwrap();
        })
        .await
        .unwrap();

        let captured = task.await.unwrap();
        assert_eq!(captured.bytes, b"abcde");
        assert!(captured.truncated);
    }

    #[tokio::test]
    async fn pipe_readers_share_one_total_capture_budget() {
        let remaining = Arc::new(AtomicUsize::new(6));
        let (mut stdout_writer, stdout_reader) = tokio::io::duplex(64);
        let (mut stderr_writer, stderr_reader) = tokio::io::duplex(64);
        let stdout_task = spawn_pipe_reader(Some(stdout_reader), Arc::clone(&remaining));
        let stderr_task = spawn_pipe_reader(Some(stderr_reader), remaining);

        let stdout_write = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            stdout_writer.write_all(b"stdout").await.unwrap();
        });
        let stderr_write = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            stderr_writer.write_all(b"stderr").await.unwrap();
        });
        stdout_write.await.unwrap();
        stderr_write.await.unwrap();

        let (stdout, stderr) = join_output(stdout_task, stderr_task).await;
        assert_eq!(stdout.bytes.len() + stderr.bytes.len(), 6);
        assert!(stdout.truncated || stderr.truncated);
    }

    #[tokio::test]
    async fn printf_format_starting_with_dash_succeeds() {
        let tool = BashTool {
            timeout_secs: 1,
            max_output_bytes: 1024,
            ..Default::default()
        };

        let result = tool
            .execute(json!({"command": "printf '--- heading ---\\n'"}), &ctx())
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "--- heading ---\n");
    }

    #[tokio::test]
    async fn printf_wrapper_preserves_dash_dash_and_v() {
        let tool = BashTool {
            timeout_secs: 1,
            max_output_bytes: 1024,
            ..Default::default()
        };

        let result = tool
            .execute(
                json!({"command": "printf -- '--- one ---\\n'; printf -v label 'two'; printf '%s\\n' \"$label\""}),
                &ctx(),
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "--- one ---\ntwo\n");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn timeout_kills_background_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let tool = BashTool {
            timeout_secs: 1,
            max_output_bytes: 1024,
            ..Default::default()
        };
        let result = tool
            .execute(
                json!({
                    "command": format!("sleep 30 & echo $! > {}; wait", pid_file.display()),
                    "timeout": 100
                }),
                &ToolContext {
                    working_dir: dir.path().to_path_buf(),
                    ..ToolContext::default()
                },
            )
            .await;

        assert!(result.is_error, "command should time out");
        let pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .to_string();
        for _ in 0..20 {
            if !process_exists(&pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(&pid)
            .status();
        panic!("background sleep process survived Bash timeout: pid={pid}");
    }

    #[cfg(unix)]
    fn process_exists(pid: &str) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
