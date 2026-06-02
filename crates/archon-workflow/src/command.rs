use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandAction {
    Plan {
        task: String,
    },
    Run {
        task: String,
    },
    Status {
        run_id: String,
    },
    Resume {
        run_id: String,
    },
    Pause {
        run_id: String,
    },
    Cancel {
        run_id: String,
    },
    RestartAgent {
        run_id: String,
        stage_id: String,
    },
    ForceAccept {
        run_id: String,
        stage_id: String,
        rationale: String,
    },
    Save {
        run_id: String,
        name: String,
    },
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCommand {
    pub action: CommandAction,
}

impl WorkflowCommand {
    pub fn parse(args: &[String]) -> WorkflowResult<Self> {
        let Some(first) = args.first() else {
            return Ok(Self {
                action: CommandAction::List,
            });
        };
        let tail = &args[1..];
        let action = match first.as_str() {
            "plan" => CommandAction::Plan {
                task: join_task(tail)?,
            },
            "run" => CommandAction::Run {
                task: join_task(tail)?,
            },
            "status" => CommandAction::Status {
                run_id: required(tail, 0, "run id")?,
            },
            "resume" => CommandAction::Resume {
                run_id: required(tail, 0, "run id")?,
            },
            "pause" => CommandAction::Pause {
                run_id: required(tail, 0, "run id")?,
            },
            "cancel" => CommandAction::Cancel {
                run_id: required(tail, 0, "run id")?,
            },
            "restart-agent" | "restart-stage" => CommandAction::RestartAgent {
                run_id: required(tail, 0, "run id")?,
                stage_id: required(tail, 1, "stage id")?,
            },
            "force-accept" | "force-continue" => CommandAction::ForceAccept {
                run_id: required(tail, 0, "run id")?,
                stage_id: required(tail, 1, "stage id")?,
                rationale: required_tail(tail, 2, "rationale")?,
            },
            "save" => CommandAction::Save {
                run_id: required(tail, 0, "run id")?,
                name: required(tail, 1, "template name")?,
            },
            "list" => CommandAction::List,
            _ => CommandAction::Run {
                task: args.join(" "),
            },
        };
        Ok(Self { action })
    }
}

fn join_task(args: &[String]) -> WorkflowResult<String> {
    let task = args.join(" ");
    if task.trim().is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "workflow task is required".into(),
        ));
    }
    Ok(task)
}

fn required(args: &[String], idx: usize, label: &str) -> WorkflowResult<String> {
    args.get(idx)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing {label}")))
}

fn required_tail(args: &[String], start: usize, label: &str) -> WorkflowResult<String> {
    let value = args.get(start..).unwrap_or_default().join(" ");
    if value.trim().is_empty() {
        return Err(WorkflowError::SpecInvalid(format!("missing {label}")));
    }
    Ok(value)
}
