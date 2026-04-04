use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::types::{HookConfig, HookError, HookResult};

// ---------------------------------------------------------------------------
// HookExecutor — spawns child processes for hooks
// ---------------------------------------------------------------------------

pub struct HookExecutor;

impl HookExecutor {
    /// Execute a hook command.
    ///
    /// - Spawns the command via `sh -c` with environment variables set.
    /// - Writes the JSON payload to the child's stdin.
    /// - For **blocking** hooks: waits (with timeout), reads stdout as JSON.
    /// - For **non-blocking** hooks: spawns a background tokio task and returns `Ok(None)`.
    /// - On timeout: kills the child process (SIGKILL).
    pub async fn execute(
        config: &HookConfig,
        payload: &serde_json::Value,
        session_id: &str,
        cwd: &Path,
    ) -> Result<Option<HookResult>, HookError> {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| HookError::ParseError(format!("failed to serialize payload: {e}")))?;

        if config.blocking {
            Self::execute_blocking(config, &payload_bytes, session_id, cwd).await
        } else {
            Self::execute_non_blocking(config, payload_bytes, session_id, cwd);
            Ok(None)
        }
    }

    async fn execute_blocking(
        config: &HookConfig,
        payload_bytes: &[u8],
        session_id: &str,
        cwd: &Path,
    ) -> Result<Option<HookResult>, HookError> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&config.command)
            .current_dir(cwd)
            .env("ARCHON_SESSION_ID", session_id)
            .env("ARCHON_CWD", cwd.to_string_lossy().as_ref())
            .env("ARCHON_HOOK_TYPE", config.hook_type.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| HookError::SpawnError(format!("{}: {e}", config.command)))?;

        // Write payload to stdin
        if let Some(mut stdin) = child.stdin.take() {
            // Ignore write errors — command may not read stdin
            let _ = stdin.write_all(payload_bytes).await;
            drop(stdin);
        }

        // Wait with timeout. We use a separate async block to capture
        // ownership of `child` while still being able to kill on timeout.
        if config.timeout_ms > 0 {
            let timeout = std::time::Duration::from_millis(config.timeout_ms);

            // Take stdout/stderr handles before we move child into the
            // wait future, so we can still kill on timeout.
            let stdout_handle = child.stdout.take();
            let stderr_handle = child.stderr.take();

            let wait_fut = async {
                let status = child.wait().await?;

                let mut stdout_bytes = Vec::new();
                if let Some(mut out) = stdout_handle {
                    tokio::io::AsyncReadExt::read_to_end(&mut out, &mut stdout_bytes).await?;
                }
                let mut stderr_bytes = Vec::new();
                if let Some(mut err) = stderr_handle {
                    tokio::io::AsyncReadExt::read_to_end(&mut err, &mut stderr_bytes).await?;
                }

                Ok::<std::process::Output, std::io::Error>(std::process::Output {
                    status,
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                })
            };

            tokio::pin!(wait_fut);

            match tokio::time::timeout(timeout, &mut wait_fut).await {
                Ok(Ok(output)) => Self::process_output(config, &output),
                Ok(Err(e)) => Err(HookError::IoError(e)),
                Err(_) => {
                    // Timeout — the child is dropped here which triggers
                    // kill_on_drop (SIGKILL for hang protection).
                    Err(HookError::Timeout {
                        command: config.command.clone(),
                        timeout_ms: config.timeout_ms,
                    })
                }
            }
        } else {
            // No timeout — wait indefinitely (blocking with timeout_ms=0)
            let output = child.wait_with_output().await?;
            Self::process_output(config, &output)
        }
    }

    fn process_output(
        config: &HookConfig,
        output: &std::process::Output,
    ) -> Result<Option<HookResult>, HookError> {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(HookError::NonZeroExit {
                code: output.status.code().unwrap_or(-1),
                stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();

        if trimmed.is_empty() {
            return Ok(None);
        }

        // Try to parse as HookResult JSON
        match serde_json::from_str::<HookResult>(trimmed) {
            Ok(result) => Ok(Some(result)),
            Err(_) => {
                // Non-JSON stdout is not an error — hook just didn't return structured data
                tracing::debug!(
                    hook = %config.command,
                    "hook stdout was not valid HookResult JSON, ignoring"
                );
                Ok(None)
            }
        }
    }

    fn execute_non_blocking(
        config: &HookConfig,
        payload_bytes: Vec<u8>,
        session_id: &str,
        cwd: &Path,
    ) {
        let command = config.command.clone();
        let hook_type = config.hook_type.to_string();
        let session = session_id.to_string();
        let working_dir = cwd.to_path_buf();

        tokio::spawn(async move {
            let result = async {
                let mut child = Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&working_dir)
                    .env("ARCHON_SESSION_ID", &session)
                    .env("ARCHON_CWD", working_dir.to_string_lossy().as_ref())
                    .env("ARCHON_HOOK_TYPE", &hook_type)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()?;

                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(&payload_bytes).await;
                    drop(stdin);
                }

                child.wait().await
            }
            .await;

            match result {
                Ok(status) if !status.success() => {
                    tracing::warn!(
                        hook = %command,
                        code = status.code().unwrap_or(-1),
                        "non-blocking hook exited with error"
                    );
                }
                Err(e) => {
                    tracing::warn!(hook = %command, error = %e, "non-blocking hook failed");
                }
                _ => {}
            }
        });
    }
}
