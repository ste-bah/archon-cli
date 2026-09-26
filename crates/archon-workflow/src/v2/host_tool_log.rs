//! The tool calls the host itself watched a write session make (Issue-116).
//!
//! The required-tool proof read only the agent's own `commands_run`. Live
//! (wf-0ddadd81, review-remediate-cross-task-t-ce04f052-1-69-0) a repair
//! session called all five declared MCP tools, each answered
//! `"success": true`, and left them out of its report; the branch was
//! failed for "required tools were never exercised this run" and a good
//! patch went back to the queue.
//!
//! The session's tool guard already appends every finished call to the
//! branch's read-set sidecar (`archon_tools::workflow_read_guard`, a
//! `tool_call` record: tool name, input head, how it ended). This module
//! reads back the records appended during ONE dispatch — from the sidecar
//! length when the dispatch started — so a tool the host saw run is credited
//! whatever the report says. Only the record's `tool` counts: a Bash call
//! whose command merely names a tool is a Bash call. A call the guard
//! refused never ran and is not credited; a call that ran and failed is a
//! captured failure, which the proof accepts from the report too.
use std::path::{Path, PathBuf};

use serde_json::Value;

tokio::task_local! { static LOG: HostToolLog; }

/// The sidecar a dispatch's guard writes, and where this dispatch starts in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostToolLog {
    path: PathBuf,
    start: u64,
}

impl HostToolLog {
    /// The records appended to `path` from now on. A sidecar that does not
    /// exist yet starts at 0.
    pub fn from_now(path: PathBuf) -> Self {
        let start = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self { path, start }
    }

    /// The tool names the host saw executed since `start`, successful or not,
    /// in call order. Empty when the sidecar is unreadable: no observation is
    /// no credit, never an error.
    pub fn executed_tools(&self) -> Vec<String> {
        executed_tools(&self.path, self.start)
    }
}

fn executed_tools(path: &Path, start: u64) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    // A sidecar rewritten shorter than the start is a different file: read
    // none of it rather than credit another session's calls.
    let Some(tail) = usize::try_from(start).ok().and_then(|s| bytes.get(s..)) else {
        return Vec::new();
    };
    String::from_utf8_lossy(tail)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record.get("kind").and_then(Value::as_str) == Some("tool_call"))
        .filter(|record| {
            let status = record.get("status").and_then(Value::as_str).unwrap_or("");
            !status.trim().is_empty() && !status.starts_with("refused")
        })
        .filter_map(|record| {
            record
                .get("tool")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

/// Run `work` with `log` as the dispatch's host tool log.
pub async fn scope<T>(log: HostToolLog, work: impl std::future::Future<Output = T>) -> T {
    LOG.scope(log, work).await
}

/// The tools the host saw the current dispatch execute; empty outside a
/// [`scope`].
pub(crate) fn observed_tools() -> Vec<String> {
    LOG.try_with(HostToolLog::executed_tools)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "host_tool_log_tests.rs"]
mod tests;
