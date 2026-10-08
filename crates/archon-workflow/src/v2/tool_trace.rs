//! The files a session read and the commands it ran, taken from the tool
//! calls the host's own session trace recorded (Issue 276).
//!
//! A result that carries no agent report (a raw provider outcome) had
//! `files_read` and `commands_run` left at their empty defaults, so a session
//! that made 80 inspection calls was stored as having read nothing. These
//! records come from the trace, never from the agent. When the trace holds no
//! call, the result says the fields are not recorded: the host cannot tell
//! "nothing ran" from "nothing was captured", and an empty list would claim
//! the first.
use serde_json::{Value, json};

use super::result::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2Result,
};
use crate::llm_client_port::WorkflowAgentToolUse;

/// The value `toolTrace.filesRead` / `toolTrace.commandsRun` carry when the
/// trace recorded no tool call.
pub const NOT_RECORDED: &str = "not_recorded";

/// The most characters of a tool input kept in one command record.
const COMMAND_CHARS: usize = 240;

/// Tools whose successful call reads the one file its input names.
const FILE_READ_TOOLS: &[&str] = &["Read", "NotebookRead"];

/// Fill `files_read` and `commands_run` from `tool_uses` and stamp
/// `data.toolTrace` with what the trace held.
pub fn record_tool_trace(result: &mut WorkflowV2Result, tool_uses: &[WorkflowAgentToolUse]) {
    for tool in tool_uses {
        let status = status(&tool.output);
        match read_path(tool) {
            Some(path) if status == WorkflowV2CommandStatus::Succeeded => {
                if !result.files_read.iter().any(|file| file.path == path) {
                    result.files_read.push(WorkflowV2FileRecord {
                        path,
                        purpose: Some(format!("{} (host tool trace)", tool.tool_name)),
                    });
                }
            }
            _ => result.commands_run.push(command_record(tool, status)),
        }
    }
    let marker = if tool_uses.is_empty() {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "the host recorded no tool call for this session: files_read and commands_run are \
             not recorded, not empty",
        ));
        json!({
            "recorded": false,
            "filesRead": NOT_RECORDED,
            "commandsRun": NOT_RECORDED,
        })
    } else {
        json!({
            "recorded": true,
            "source": "host session trace",
            "toolCalls": tool_uses.len(),
        })
    };
    if !result.data.is_object() {
        result.data = json!({});
    }
    result.data["toolTrace"] = marker;
}

fn read_path(tool: &WorkflowAgentToolUse) -> Option<String> {
    if !FILE_READ_TOOLS.contains(&tool.tool_name.as_str()) {
        return None;
    }
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|key| tool.input.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
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
    WorkflowV2CommandRecord {
        kind: if tool.tool_name == "Bash" {
            WorkflowV2CommandKind::Other
        } else {
            WorkflowV2CommandKind::Inspect
        },
        command: clip(&command),
        status,
        exit_code: None,
        output_summary: output_summary.to_string(),
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
