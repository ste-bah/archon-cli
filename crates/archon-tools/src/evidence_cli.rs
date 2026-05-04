//! Shared runner for Evidence Engine CLI-backed tools.
//!
//! These tools live in `archon-tools`, while the real document, provenance and
//! learning implementations live in the binary/crate layers above it. Running
//! the current `archon` executable keeps the tool surface wired to the same CLI
//! machinery users exercise, without introducing dependency cycles.

use std::path::PathBuf;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::tool::{ToolContext, ToolResult};

pub async fn run_archon(args: Vec<String>, ctx: &ToolContext) -> ToolResult {
    let bin = match archon_bin() {
        Ok(bin) => bin,
        Err(e) => return ToolResult::error(e),
    };
    let mut command = Command::new(bin);
    command.args(&args).env("ARCHON_TOOL_CHILD", "1");
    if !ctx.working_dir.as_os_str().is_empty() {
        command.current_dir(&ctx.working_dir);
    }
    let output = command.output().await;
    let output = match output {
        Ok(output) => output,
        Err(e) => return ToolResult::error(format!("failed to run archon {:?}: {e}", args)),
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        return ToolResult::success(join_output(stdout, stderr));
    }
    ToolResult::error(join_output(stdout, stderr))
}

fn archon_bin() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ARCHON_EVIDENCE_TOOL_BIN") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().map_err(|e| format!("cannot locate current archon executable: {e}"))
}

fn join_output(stdout: String, stderr: String) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => "(no output)".to_string(),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

pub fn required_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{key} is required and must be a non-empty string"))
}

pub fn opt_string(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

pub fn opt_bool(input: &Value, key: &str) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn opt_usize(input: &Value, key: &str, default: usize) -> Result<usize, String> {
    let Some(value) = input.get(key) else {
        return Ok(default);
    };
    let Some(raw) = value.as_u64() else {
        return Err(format!("{key} must be an integer"));
    };
    usize::try_from(raw).map_err(|_| format!("{key} is too large"))
}

pub fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}
