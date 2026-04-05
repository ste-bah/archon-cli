use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

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

pub struct BashTool {
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_secs: 120,
            max_output_bytes: 102400,
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
                    "description": "Optional timeout in milliseconds"
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

        let timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.timeout_secs * 1000);

        // Build sanitized environment
        let env_vars = sanitized_env();

        let mut child = match Command::new("/bin/bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .process_group(0) // new process group for clean kill
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to spawn bash: {e}")),
        };

        // Read output with timeout
        let timeout = Duration::from_millis(timeout_ms);
        let result = tokio::time::timeout(timeout, async {
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
                        content: format!("Exit code {exit_code}\n{output}"),
                        is_error: true,
                    }
                } else {
                    ToolResult::success(output)
                }
            }
            Err(_) => {
                // Timeout -- kill the process group
                let _ = child.kill().await;
                ToolResult::error(format!("Command timed out after {}ms", timeout_ms))
            }
        }
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

        // Use the permission classifier
        match archon_permissions::classifier::classify_command(command, &[], &[], &[]) {
            archon_permissions::classifier::CommandClass::Safe => PermissionLevel::Safe,
            archon_permissions::classifier::CommandClass::Risky => PermissionLevel::Risky,
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
        }
    }
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
