use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct PowerShellTool {
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
}

impl Default for PowerShellTool {
    fn default() -> Self {
        Self {
            timeout_secs: 120,
            max_output_bytes: 102400,
        }
    }
}

#[async_trait::async_trait]
impl Tool for PowerShellTool {
    fn name(&self) -> &str {
        "PowerShell"
    }

    fn description(&self) -> &str {
        "Executes a PowerShell command and returns its output."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The PowerShell command to execute"
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

        // Check if pwsh is available
        let shell = if which_pwsh() { "pwsh" } else { "powershell" };

        // Build sanitized environment (same as BashTool)
        let env_vars = crate::bash::sanitized_env();

        let mut child = match Command::new(shell)
            .arg("-Command")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .env_clear()
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(format!(
                    "PowerShell not available ({shell}): {e}"
                ))
            }
        };

        let timeout = Duration::from_secs(self.timeout_secs);
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
                let mut output = String::from_utf8_lossy(&stdout_buf).to_string();
                let stderr_str = String::from_utf8_lossy(&stderr_buf);
                if !stderr_str.is_empty() {
                    output.push_str(&stderr_str);
                }

                if output.len() > self.max_output_bytes {
                    output.truncate(self.max_output_bytes);
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
                let _ = child.kill().await;
                ToolResult::error("PowerShell command timed out")
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

fn which_pwsh() -> bool {
    std::process::Command::new("which")
        .arg("pwsh")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
