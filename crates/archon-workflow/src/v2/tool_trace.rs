//! The files a session read and the commands it ran, taken from the tool
//! calls the host's own session trace recorded (Issue 276).
//!
//! A result that carries no agent report (a raw provider outcome) had
//! `files_read` and `commands_run` left at their empty defaults, so a session
//! that made 80 inspection calls was stored as having read nothing. These
//! records come from the trace, never from the agent.
//!
//! When an empty list is the truth: the host's session trace ends with a
//! summary entry ([`TOOL_TRACE_SUMMARY_NAME`]) that is written only when the
//! session's history was captured. That history holds every `tool_use`
//! block the session made, so a summary that counts zero calls proves the
//! session made none, and empty lists are then stored as fact. With no
//! summary and no call, the host cannot tell "nothing ran" from "nothing was
//! captured", so both fields are marked [`NOT_RECORDED`] instead.
//!
//! Every string kept here passes the repository's secret redaction first: a
//! tool input is agent-written and can carry a credential. Only call names,
//! inputs and the error flag are read; a tool's output is never stored.
use archon_observability::redaction::redact_text;
use archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
use serde_json::{Value, json};

use super::result::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2Result,
};
use crate::llm_client_port::WorkflowAgentToolUse;

/// The value `toolTrace.filesRead` / `toolTrace.commandsRun` carry when no
/// trace was recorded.
pub const NOT_RECORDED: &str = "not_recorded";

/// The most characters of a tool input kept in one command record.
const COMMAND_CHARS: usize = 240;

/// Tools whose successful call reads the one file its input names.
const FILE_READ_TOOLS: &[&str] = &["Read", "NotebookRead"];

/// Fill `files_read` and `commands_run` from `tool_uses` and stamp
/// `data.toolTrace` with what the trace held.
pub fn record_tool_trace(result: &mut WorkflowV2Result, tool_uses: &[WorkflowAgentToolUse]) {
    let summary = tool_uses
        .iter()
        .rev()
        .find(|tool| tool.tool_name == TOOL_TRACE_SUMMARY_NAME)
        .map(|tool| &tool.input);
    let calls: Vec<_> = tool_uses
        .iter()
        .filter(|tool| tool.tool_name != TOOL_TRACE_SUMMARY_NAME)
        .collect();
    let mut seen = Vec::new();
    for tool in &calls {
        let status = status(&tool.output);
        match read_path(tool) {
            Some(path) if status == WorkflowV2CommandStatus::Succeeded => {
                if !seen.contains(&path) {
                    result.files_read.push(WorkflowV2FileRecord {
                        path: redact_text(&path),
                        purpose: Some(format!(
                            "{} (host tool trace)",
                            redact_text(&tool.tool_name)
                        )),
                    });
                    seen.push(path);
                }
            }
            _ => result.commands_run.push(command_record(tool, status)),
        }
    }
    let marker = match summary {
        None if calls.is_empty() => {
            result.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "the host recorded no tool trace for this session: files_read and commands_run \
                 are not recorded, not empty",
            ));
            json!({
                "recorded": false,
                "filesRead": NOT_RECORDED,
                "commandsRun": NOT_RECORDED,
            })
        }
        None => json!({
            "recorded": true,
            "source": "agent outcome tool uses",
            "toolCalls": calls.len(),
            "complete": false,
        }),
        Some(summary) => {
            let count = |key: &str| summary.get(key).and_then(Value::as_u64).unwrap_or(0);
            let (dropped, cut) = (count("dropped"), count("inputs_truncated"));
            if dropped > 0 {
                result.evidence.push(WorkflowV2Evidence::new(
                    WorkflowV2EvidenceKind::Inspection,
                    format!(
                        "the host session trace held {} tool calls and kept the first {}: \
                         files_read and commands_run cover only those",
                        count("calls"),
                        calls.len()
                    ),
                ));
            }
            json!({
                "recorded": true,
                "source": "host session trace",
                "toolCalls": count("calls"),
                "kept": calls.len(),
                "dropped": dropped,
                "inputsTruncated": cut,
                "complete": dropped == 0,
            })
        }
    };
    if !result.data.is_object() {
        result.data = json!({});
    }
    result.data["toolTrace"] = marker;
}

/// The file a read call names, unless the trace cut its input: a cut path is
/// not the path that was read.
fn read_path(tool: &WorkflowAgentToolUse) -> Option<String> {
    if !FILE_READ_TOOLS.contains(&tool.tool_name.as_str()) || input_truncated(tool) {
        return None;
    }
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|key| tool.input.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn input_truncated(tool: &WorkflowAgentToolUse) -> bool {
    tool.output.get("input_truncated").and_then(Value::as_bool) == Some(true)
}

/// A call with a result is a success unless the result was an error; a call
/// the trace holds no result for is not claimed to have run to completion.
fn status(output: &Value) -> WorkflowV2CommandStatus {
    match output.get("is_error").and_then(Value::as_bool) {
        Some(true) => WorkflowV2CommandStatus::Failed,
        Some(false) => WorkflowV2CommandStatus::Succeeded,
        None => WorkflowV2CommandStatus::Skipped,
    }
}

fn command_record(
    tool: &WorkflowAgentToolUse,
    status: WorkflowV2CommandStatus,
) -> WorkflowV2CommandRecord {
    let shell = tool.input.get("command").and_then(Value::as_str);
    let command = match (tool.tool_name.as_str(), shell) {
        ("Bash", Some(command)) => command.to_string(),
        _ => format!("{} {}", tool.tool_name, tool.input),
    };
    let output_summary = match status {
        WorkflowV2CommandStatus::Succeeded => "host tool trace: call returned",
        WorkflowV2CommandStatus::Failed => "host tool trace: call returned an error",
        WorkflowV2CommandStatus::Skipped => "host tool trace: no result recorded for this call",
    };
    let output_summary = if input_truncated(tool) {
        format!("{output_summary}; input cut by the trace bound")
    } else {
        output_summary.to_string()
    };
    WorkflowV2CommandRecord {
        kind: if tool.tool_name == "Bash" {
            WorkflowV2CommandKind::Other
        } else {
            WorkflowV2CommandKind::Inspect
        },
        // Redacted before it is clipped: a cut could split a credential
        // into a fragment the redaction no longer recognises.
        command: clip(&redact_text(&command)),
        status,
        exit_code: None,
        output_summary,
        pre_existing: false,
    }
}

fn clip(text: &str) -> String {
    if text.chars().count() <= COMMAND_CHARS {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(COMMAND_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

#[cfg(test)]
#[path = "tool_trace_tests.rs"]
mod tests;
