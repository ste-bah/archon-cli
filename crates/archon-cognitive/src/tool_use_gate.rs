use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::types::{Situation, SituationKind, ToolVerdict};

#[derive(Debug, Clone)]
pub struct ToolGateInput<'a> {
    pub situation: &'a Situation,
    pub tool_name: &'a str,
    pub tool_input: &'a Value,
    pub working_dir: &'a Path,
}

#[derive(Debug, Default, Clone)]
pub struct ToolUseGate;

impl ToolUseGate {
    pub fn evaluate(&self, input: ToolGateInput<'_>) -> ToolVerdict {
        if let Some(verdict) = git_probe_context_note(&input) {
            return verdict;
        }

        match input.situation.kind {
            SituationKind::Greeting => ToolVerdict::Suppress {
                reason: "trivial turn must be answered directly without tools".to_owned(),
            },
            SituationKind::SimpleQuestion => simple_question_verdict(input.tool_name),
            SituationKind::Ambiguous => ambiguous_verdict(input.tool_name),
            SituationKind::CiDebug => ci_debug_verdict(input.tool_name, input.tool_input),
            _ => ToolVerdict::Allow {
                reason: format!("{} permits tool use", input.situation.kind.as_str()),
            },
        }
    }
}

fn simple_question_verdict(tool_name: &str) -> ToolVerdict {
    if matches!(tool_name, "memory_recall" | "MemoryRecall") {
        ToolVerdict::Allow {
            reason: "simple question allows targeted memory recall".to_owned(),
        }
    } else {
        ToolVerdict::Suppress {
            reason: "simple question did not justify external tools".to_owned(),
        }
    }
}

fn ambiguous_verdict(tool_name: &str) -> ToolVerdict {
    if matches!(tool_name, "AskUserQuestion" | "AskUser") {
        ToolVerdict::Allow {
            reason: "ambiguous turn may ask a clarification".to_owned(),
        }
    } else {
        ToolVerdict::Suppress {
            reason: "ambiguous turn must clarify before tool use".to_owned(),
        }
    }
}

fn ci_debug_verdict(tool_name: &str, input: &Value) -> ToolVerdict {
    if tool_name != "Bash" {
        return ToolVerdict::Allow {
            reason: "CI debug permits non-mutating diagnostic tools".to_owned(),
        };
    }
    let command = command_text(input).unwrap_or_default();
    if command.contains("gh ") || command.contains("git ") {
        ToolVerdict::Allow {
            reason: "CI debug permits gh/git diagnostics".to_owned(),
        }
    } else {
        ToolVerdict::Suppress {
            reason: "CI debug Bash command must be gh/git diagnostic".to_owned(),
        }
    }
}

fn git_probe_context_note(input: &ToolGateInput<'_>) -> Option<ToolVerdict> {
    if !matches!(input.tool_name, "Bash" | "PowerShell") {
        return None;
    }
    let command = command_text(input.tool_input)?;
    if !is_git_probe(command) || is_git_repo(input.working_dir) {
        return None;
    }
    Some(ToolVerdict::ConvertToContextNote {
        note: "Skipped git repository probe: current directory is not a git repository.".to_owned(),
    })
}

fn command_text(input: &Value) -> Option<&str> {
    input.get("command").and_then(Value::as_str)
}

fn is_git_probe(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.starts_with("git rev-parse")
        || trimmed.starts_with("git status")
        || trimmed.starts_with("git branch")
        || trimmed.starts_with("git log")
}

fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}
