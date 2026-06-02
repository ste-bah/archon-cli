use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandAction {
    Plan {
        task: String,
    },
    PlanSpec {
        path: String,
    },
    Run {
        task: String,
    },
    RunSpec {
        path: String,
    },
    RunTemplate {
        name: String,
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
            "plan" => parse_plan(tail)?,
            "plan-spec" => CommandAction::PlanSpec {
                path: required(tail, 0, "spec file")?,
            },
            "run" => parse_run(tail)?,
            "run-spec" => CommandAction::RunSpec {
                path: required(tail, 0, "spec file")?,
            },
            "run-template" | "from-template" => CommandAction::RunTemplate {
                name: required(tail, 0, "template name")?,
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

fn parse_plan(args: &[String]) -> WorkflowResult<CommandAction> {
    if flag(args, "--spec-file") {
        return Ok(CommandAction::PlanSpec {
            path: required(args, 1, "spec file")?,
        });
    }
    Ok(CommandAction::Plan {
        task: join_task(args)?,
    })
}

fn parse_run(args: &[String]) -> WorkflowResult<CommandAction> {
    if flag(args, "--spec-file") {
        return Ok(CommandAction::RunSpec {
            path: required(args, 1, "spec file")?,
        });
    }
    if flag(args, "--from-template") {
        return Ok(CommandAction::RunTemplate {
            name: required(args, 1, "template name")?,
        });
    }
    Ok(CommandAction::Run {
        task: join_task(args)?,
    })
}

fn flag(args: &[String], expected: &str) -> bool {
    args.first().is_some_and(|value| value == expected)
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

#[cfg(test)]
mod tests {
    use super::{CommandAction, WorkflowCommand};

    fn parse(args: &[&str]) -> CommandAction {
        WorkflowCommand::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
            .unwrap()
            .action
    }

    #[test]
    fn parses_spec_and_template_entry_points() {
        assert_eq!(
            parse(&["plan", "--spec-file", "workflow.yaml"]),
            CommandAction::PlanSpec {
                path: "workflow.yaml".into()
            }
        );
        assert_eq!(
            parse(&["run", "--spec-file", "workflow.yaml"]),
            CommandAction::RunSpec {
                path: "workflow.yaml".into()
            }
        );
        assert_eq!(
            parse(&["run", "--from-template", "repo-audit"]),
            CommandAction::RunTemplate {
                name: "repo-audit".into()
            }
        );
    }
}
