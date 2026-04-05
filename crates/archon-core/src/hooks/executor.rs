use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::types::{HookConfig, HookResult};

// ---------------------------------------------------------------------------
// Internal result from running a shell command
// ---------------------------------------------------------------------------

struct CommandOutput {
    exit_code: i32,
    stderr: String,
}

// ---------------------------------------------------------------------------
// Internal error (never propagated — hooks always return HookResult)
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RunError {
    Spawn(String),
    Io(String),
    Timeout,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "spawn error: {s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
            Self::Timeout => write!(f, "timed out"),
        }
    }
}

// ---------------------------------------------------------------------------
// Public hook executor
// ---------------------------------------------------------------------------

/// Execute a single `HookConfig` command.
///
/// **Exit code semantics:**
/// - `0` → `HookResult::Allow`
/// - `2` → `HookResult::Block { reason }` where `reason` is the hook's stderr
/// - Any other code or error (timeout, spawn failure) → `HookResult::Allow` (logged)
///
/// If `config.async == Some(true)`, the command is spawned in the background
/// and `HookResult::Allow` is returned immediately without waiting.
pub(crate) async fn execute_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    // Async: fire-and-forget, return Allow immediately.
    if config.r#async == Some(true) {
        spawn_background(
            config.command.clone(),
            input.clone(),
            cwd.to_path_buf(),
            session_id.to_owned(),
            event_name.to_owned(),
            config.timeout.unwrap_or(60),
        );
        return HookResult::Allow;
    }

    let payload_bytes = match serde_json::to_vec(input) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "hook: failed to serialize input payload");
            return HookResult::Allow;
        }
    };

    let timeout_secs = config.timeout.unwrap_or(60);

    match run_command(
        &config.command,
        &payload_bytes,
        cwd,
        session_id,
        event_name,
        timeout_secs,
    )
    .await
    {
        Ok(output) => interpret_exit_code(&config.command, output),
        Err(e) => {
            tracing::warn!(
                hook = %config.command,
                error = %e,
                "hook execution failed (non-blocking, returning Allow)"
            );
            HookResult::Allow
        }
    }
}

// ---------------------------------------------------------------------------
// Exit code interpretation
// ---------------------------------------------------------------------------

fn interpret_exit_code(command: &str, output: CommandOutput) -> HookResult {
    match output.exit_code {
        0 => HookResult::Allow,
        2 => {
            let reason = if output.stderr.trim().is_empty() {
                format!("hook '{command}' blocked tool execution (exit 2)")
            } else {
                output.stderr.trim().to_owned()
            };
            HookResult::Block { reason }
        }
        code => {
            tracing::warn!(
                hook = %command,
                exit_code = code,
                stderr = %output.stderr.trim(),
                "hook exited with non-zero code (non-blocking failure, returning Allow)"
            );
            HookResult::Allow
        }
    }
}

// ---------------------------------------------------------------------------
// Shell command runner
// ---------------------------------------------------------------------------

async fn run_command(
    command: &str,
    payload_bytes: &[u8],
    cwd: &Path,
    session_id: &str,
    event_name: &str,
    timeout_secs: u32,
) -> Result<CommandOutput, RunError> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("ARCHON_SESSION_ID", session_id)
        .env("ARCHON_CWD", cwd.to_string_lossy().as_ref())
        .env("ARCHON_HOOK_EVENT", event_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| RunError::Spawn(format!("{command}: {e}")))?;

    // Write payload to stdin then drop so the child gets EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_bytes).await;
    }

    let timeout = std::time::Duration::from_secs(u64::from(timeout_secs));

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            Ok(CommandOutput { exit_code, stderr })
        }
        Ok(Err(e)) => Err(RunError::Io(e.to_string())),
        Err(_) => Err(RunError::Timeout),
    }
}

// ---------------------------------------------------------------------------
// Background (async: true) execution
// ---------------------------------------------------------------------------

fn spawn_background(
    command: String,
    input: serde_json::Value,
    cwd: std::path::PathBuf,
    session_id: String,
    event_name: String,
    timeout_secs: u32,
) {
    tokio::spawn(async move {
        let payload_bytes = match serde_json::to_vec(&input) {
            Ok(b) => b,
            Err(_) => return,
        };
        let _ = run_command(
            &command,
            &payload_bytes,
            &cwd,
            &session_id,
            &event_name,
            timeout_secs,
        )
        .await;
    });
}
