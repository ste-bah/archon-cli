//! TASK-P0-B.6a Monitor tool.
//!
//! Bounded-time polling wrapper that runs a command and collects its
//! stdout within a time window. Unlike [`crate::bash::BashTool`],
//! `Monitor` returns BEFORE the command exits if `timeout_ms` elapses —
//! useful for tailing logs or polling APIs without blocking an entire
//! turn.
//!
//! Each line of stdout is treated as a discrete "event". On return,
//! the tool reports the collected output plus whether the process
//! exited cleanly, was still running at timeout, or crashed.
//!
//! The output is serialized as a pretty-printed JSON string so the LLM
//! can parse it structurally:
//!
//! ```json
//! {
//!   "exit": "exited" | "timeout" | "error",
//!   "code": <i32>,
//!   "events": ["line 1", "line 2", ...],
//!   "truncated": <bool>
//! }
//! ```
//!
//! Permission level mirrors [`BashTool`](crate::bash::BashTool): we
//! delegate to `archon_permissions::classifier::classify_command` so
//! the same allow/deny logic applies.

use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Default collection window in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 3_000;

/// Hard cap for `timeout_ms` to keep a misbehaving model from stalling
/// the turn indefinitely.
const MAX_TIMEOUT_MS: u64 = 30_000;

/// Maximum number of stdout lines captured before truncation.
const MAX_EVENTS: usize = 200;

pub struct MonitorTool;

#[async_trait::async_trait]
impl Tool for MonitorTool {
    fn name(&self) -> &str {
        "Monitor"
    }

    fn description(&self) -> &str {
        "Run a shell command and collect its stdout as line-level events \
         within a bounded time window. Unlike Bash, Monitor returns BEFORE \
         the command exits if timeout_ms elapses — useful for tailing logs, \
         polling APIs, or watching a background process without blocking \
         the turn. Returns a JSON object: { exit: 'exited' | 'timeout' | \
         'error', code: i32, events: [line, ...], truncated: bool }."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to poll."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Max collection window in milliseconds (default 3000, cap 30000)."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c,
            Some(_) => return ToolResult::error("command must be a non-empty string"),
            None => return ToolResult::error("command is required and must be a string"),
        };

        let timeout_ms = input
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        run_monitor(command, timeout_ms, ctx).await
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        // Mirror BashTool: delegate to the shared command classifier so
        // the same allow/deny rules apply to both tools.
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        match archon_permissions::classifier::classify_command(command, &[], &[], &[]) {
            archon_permissions::classifier::CommandClass::Safe => PermissionLevel::Safe,
            archon_permissions::classifier::CommandClass::Risky => PermissionLevel::Risky,
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
        }
    }
}

/// Spawn the shell command with piped stdout, read lines until the
/// timeout fires or the child exits, then assemble the JSON report.
async fn run_monitor(command: &str, timeout_ms: u64, ctx: &ToolContext) -> ToolResult {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(not(unix))]
    let mut cmd = {
        // Fall back to the system shell on non-unix; the tool's tests
        // are #[cfg(unix)] so this branch is exercised only in prod
        // builds on Windows.
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };

    cmd.current_dir(&ctx.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::success(format_report(
                "error",
                -1,
                &[],
                false,
                Some(&e.to_string()),
            ));
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill().await;
            return ToolResult::success(format_report(
                "error",
                -1,
                &[],
                false,
                Some("failed to capture stdout"),
            ));
        }
    };

    let mut reader = BufReader::new(stdout).lines();
    let mut events: Vec<String> = Vec::new();
    let mut truncated = false;

    let timeout = Duration::from_millis(timeout_ms);
    let collect = async {
        loop {
            tokio::select! {
                line = reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            if events.len() < MAX_EVENTS {
                                events.push(l);
                            } else {
                                truncated = true;
                                // Keep reading to drain the pipe but
                                // drop the line so memory stays bounded.
                            }
                        }
                        Ok(None) => break, // EOF — child closed stdout
                        Err(_) => break,
                    }
                }
                status = child.wait() => {
                    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
                    // Drain any remaining buffered lines.
                    while let Ok(Some(l)) = reader.next_line().await {
                        if events.len() < MAX_EVENTS {
                            events.push(l);
                        } else {
                            truncated = true;
                        }
                    }
                    return ("exited", code);
                }
            }
        }
        // Reader hit EOF before wait — poll the child for its status.
        let code = match child.wait().await {
            Ok(s) => s.code().unwrap_or(-1),
            Err(_) => -1,
        };
        ("exited", code)
    };

    match tokio::time::timeout(timeout, collect).await {
        Ok((exit, code)) => {
            ToolResult::success(format_report(exit, code, &events, truncated, None))
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            ToolResult::success(format_report("timeout", -1, &events, truncated, None))
        }
    }
}

/// Serialize the monitor result as pretty-printed JSON.
fn format_report(
    exit: &str,
    code: i32,
    events: &[String],
    truncated: bool,
    error: Option<&str>,
) -> String {
    let mut value = json!({
        "exit": exit,
        "code": code,
        "events": events,
        "truncated": truncated,
    });
    if let Some(msg) = error {
        value["error"] = json!(msg);
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| {
        "{\"exit\":\"error\",\"code\":-1,\"events\":[],\"truncated\":false}".to_string()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            ..Default::default()
        }
    }

    fn parse(result: &ToolResult) -> Value {
        assert!(
            !result.is_error,
            "expected success, got error: {}",
            result.content
        );
        serde_json::from_str(&result.content).expect("tool must emit JSON")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn monitor_echo_captures_line() {
        let tool = MonitorTool;
        let input = json!({ "command": "echo hello", "timeout_ms": 2000 });
        let result = tool.execute(input, &ctx()).await;
        let v = parse(&result);
        assert_eq!(v["exit"], "exited");
        assert_eq!(v["code"], 0);
        let events: Vec<String> = v["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_string())
            .collect();
        assert!(
            events.iter().any(|l| l.contains("hello")),
            "events: {events:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn monitor_sleep_hits_timeout() {
        let tool = MonitorTool;
        let input = json!({ "command": "sleep 5", "timeout_ms": 100 });
        let result = tool.execute(input, &ctx()).await;
        let v = parse(&result);
        assert_eq!(
            v["exit"], "timeout",
            "expected timeout, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn monitor_missing_command_errors() {
        let tool = MonitorTool;
        let result = tool.execute(json!({}), &ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("command"));
    }

    #[tokio::test]
    async fn monitor_empty_command_errors() {
        let tool = MonitorTool;
        let result = tool.execute(json!({ "command": "" }), &ctx()).await;
        assert!(result.is_error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn monitor_multi_line_output() {
        let tool = MonitorTool;
        let input = json!({
            "command": "printf 'one\\ntwo\\nthree\\n'",
            "timeout_ms": 2000,
        });
        let result = tool.execute(input, &ctx()).await;
        let v = parse(&result);
        assert_eq!(v["exit"], "exited");
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), 3, "expected 3 lines, got: {events:?}");
        assert_eq!(events[0], "one");
        assert_eq!(events[1], "two");
        assert_eq!(events[2], "three");
    }

    #[test]
    fn permission_level_matches_bash_classifier() {
        let tool = MonitorTool;
        // A clearly dangerous command should be classified Dangerous,
        // mirroring BashTool's behavior through the shared classifier.
        let level = tool.permission_level(&json!({ "command": "rm -rf /" }));
        assert_eq!(level, PermissionLevel::Dangerous);
    }

    #[test]
    fn input_schema_requires_command() {
        let tool = MonitorTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "command"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn monitor_caps_events_and_marks_truncated() {
        // Produce more than MAX_EVENTS lines; the tool must cap them
        // and report truncated=true.
        let tool = MonitorTool;
        let cmd = format!(
            "i=0; while [ $i -lt {} ]; do echo $i; i=$((i+1)); done",
            MAX_EVENTS + 20
        );
        let input = json!({ "command": cmd, "timeout_ms": 5000 });
        let result = tool.execute(input, &ctx()).await;
        let v = parse(&result);
        assert_eq!(v["exit"], "exited");
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(v["truncated"], true);
    }
}
