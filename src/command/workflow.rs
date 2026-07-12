use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use archon_workflow::run::StageState;
use archon_workflow::{
    CommandAction, HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, RunStatus,
    StageStatus, TemplateRegistry, WorkflowApprovalStore, WorkflowBundle, WorkflowBundleOrigin,
    WorkflowCommand, WorkflowCommandRegistry, WorkflowPlanner, WorkflowRun, WorkflowSpec,
    WorkflowStore, WorkflowV2CallExecution, WorkflowV2ResultStore,
};

use crate::cli_args::WorkflowAction;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::workflow_live::{run_live_cli_action, should_spawn_live, spawn_live_workflow};

pub(crate) struct WorkflowHandler;

impl CommandHandler for WorkflowHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let command = WorkflowCommand::parse(args)?;
        let cwd = ctx
            .working_dir
            .clone()
            .ok_or_else(|| anyhow!("workflow command requires working directory context"))?;
        if should_spawn_live(&command.action)
            && let Some(llm) = ctx.llm_adapter.clone()
        {
            spawn_live_workflow(
                cwd,
                command.action,
                llm,
                ctx.tui_tx.clone(),
                ctx.config_path.clone(),
            );
            ctx.emit(TuiEvent::SlashCommandComplete);
            return Ok(());
        }
        if matches!(
            command.action,
            CommandAction::List | CommandAction::Status { .. }
        ) && emit_workflow_rows(&cwd, &command.action, ctx)?
        {
            ctx.emit(TuiEvent::SlashCommandComplete);
            return Ok(());
        }
        let output = run_action(&cwd, command.action)?;
        ctx.emit(TuiEvent::TextDelta(output));
        ctx.emit(TuiEvent::SlashCommandComplete);
        Ok(())
    }

    fn description(&self) -> &str {
        "Plan, run, resume, and inspect dynamic workflows"
    }
}

pub(crate) async fn handle_workflow_command(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let (action, mode) = cli_action(action)?;
    let output = match mode {
        CliExecutionMode::Deterministic => run_action(&cwd, action)?,
        CliExecutionMode::Live => run_live_cli_action(&cwd, action, config, env_vars).await?,
    };
    println!("{output}");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliExecutionMode {
    Deterministic,
    Live,
}

fn cli_action(action: &WorkflowAction) -> Result<(CommandAction, CliExecutionMode)> {
    let converted = match action {
        WorkflowAction::Plan {
            spec_file,
            live,
            task,
        } => {
            if let Some(path) = spec_file {
                ensure_no_task(task, "--spec-file")?;
                return Ok((
                    CommandAction::PlanSpec {
                        path: path.display().to_string(),
                    },
                    CliExecutionMode::Deterministic,
                ));
            }
            return Ok((
                CommandAction::Plan {
                    task: task_string(task)?,
                },
                mode(*live),
            ));
        }
        WorkflowAction::Run {
            spec_file,
            from_template,
            resume_from,
            live,
            yes,
            task,
        } => {
            require_live_approval(*live, *yes, "workflow run --live")?;
            if let Some(run_id) = resume_from {
                ensure_resume_from_compatible(spec_file, from_template)?;
                return Ok((
                    CommandAction::Resume {
                        run_id: run_id.clone(),
                    },
                    mode(*live),
                ));
            }
            let action = run_cli_action(spec_file.as_ref(), from_template.as_ref(), task)?;
            return Ok((action, mode(*live)));
        }
        WorkflowAction::Status { run_id } => CommandAction::Status {
            run_id: run_id.clone(),
        },
        WorkflowAction::Resume { live, yes, run_id } => {
            require_live_approval(*live, *yes, "workflow resume --live")?;
            return Ok((
                CommandAction::Resume {
                    run_id: run_id.clone(),
                },
                mode(*live),
            ));
        }
        WorkflowAction::Continue { live, yes, run_id } => {
            require_live_approval(*live, *yes, "workflow continue --live")?;
            return Ok((
                CommandAction::Continue {
                    run_id: run_id.clone(),
                },
                mode(*live),
            ));
        }
        WorkflowAction::Repair { run_id } => CommandAction::Repair {
            run_id: run_id.clone(),
        },
        WorkflowAction::Pause { run_id } => CommandAction::Pause {
            run_id: run_id.clone(),
        },
        WorkflowAction::Cancel { run_id } => CommandAction::Cancel {
            run_id: run_id.clone(),
        },
        WorkflowAction::ApproveRunOnce { run_id } => CommandAction::ApproveRunOnce {
            run_id: run_id.clone(),
        },
        WorkflowAction::ApproveAlways { run_id } => CommandAction::ApproveAlways {
            run_id: run_id.clone(),
        },
        WorkflowAction::DenyWorkflow { run_id } => CommandAction::DenyWorkflow {
            run_id: run_id.clone(),
        },
        WorkflowAction::RestartAgent {
            run_id,
            stage_id,
            item,
        } => CommandAction::RestartAgent {
            run_id: run_id.clone(),
            stage_id: stage_id.clone(),
            item: item.clone(),
        },
        WorkflowAction::RestartStage { run_id, stage_id } => CommandAction::RestartStage {
            run_id: run_id.clone(),
            stage_id: stage_id.clone(),
        },
        WorkflowAction::RestartTask { run_id, task_id } => CommandAction::RestartTask {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
        },
        WorkflowAction::ForceAccept {
            run_id,
            stage_id,
            rationale,
        } => CommandAction::ForceAccept {
            run_id: run_id.clone(),
            stage_id: stage_id.clone(),
            rationale: task_string(rationale)?,
        },
        WorkflowAction::Save { run_id, name } => CommandAction::Save {
            run_id: run_id.clone(),
            name: name.clone(),
        },
        WorkflowAction::List => CommandAction::List,
    };
    Ok((converted, CliExecutionMode::Deterministic))
}

pub(super) fn run_action(cwd: &Path, action: CommandAction) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let planner = HeuristicWorkflowPlanner;
    let text = match action {
        CommandAction::Plan { task } => planner.plan(&task)?.to_yaml()?,
        CommandAction::PlanSpec { path } => load_spec_file(cwd, &path)?.to_yaml()?,
        CommandAction::Run { .. }
        | CommandAction::RunSpec { .. }
        | CommandAction::RunTemplate { .. }
        | CommandAction::Resume { .. }
        | CommandAction::Continue { .. } => {
            return Err(anyhow!(
                "legacy deterministic workflow execution was removed by the workflow runtime                  rescue; workflows run through the live V2 runtime"
            ));
        }
        CommandAction::Status { run_id } => status_detail_text(&store, &run_id)?,
        CommandAction::Repair { run_id } => repair_workflow(&store, &run_id)?,
        CommandAction::Pause { run_id } => lifecycle(&store, &run_id, LifecycleAction::Pause)?,
        CommandAction::Cancel { run_id } => lifecycle(&store, &run_id, LifecycleAction::Cancel)?,
        CommandAction::ApproveRunOnce { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::RunOnce)?
        }
        CommandAction::ApproveAlways { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::Always)?
        }
        CommandAction::DenyWorkflow { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::Deny)?
        }
        CommandAction::RestartAgent {
            run_id,
            stage_id,
            item,
        } => match item {
            Some(item_id) => lifecycle(
                &store,
                &run_id,
                LifecycleAction::RestartItem { stage_id, item_id },
            )?,
            None => lifecycle(&store, &run_id, LifecycleAction::RestartStage(stage_id))?,
        },
        CommandAction::RestartStage { run_id, stage_id } => {
            lifecycle(&store, &run_id, LifecycleAction::RestartStage(stage_id))?
        }
        CommandAction::RestartTask { run_id, task_id } => {
            restart_task_workflow(&store, &run_id, &task_id)?
        }
        CommandAction::ForceAccept {
            run_id,
            stage_id,
            rationale,
        } => lifecycle(
            &store,
            &run_id,
            LifecycleAction::ForceAcceptStage {
                stage_id,
                forced_by: "workflow-command".to_string(),
                rationale,
                source: "cli_or_tui".to_string(),
            },
        )?,
        CommandAction::Save { run_id, name } => {
            let run = store.load_state(&run_id)?;
            let command = WorkflowCommandRegistry::project(cwd).save_run(&name, &store, &run)?;
            format!(
                "Workflow command saved: {} ({})",
                command.name,
                command.command_dir.display()
            )
        }
        CommandAction::List => list_text(&store)?,
    };
    Ok(text)
}

include!("workflow_spec_execution.rs");

include!("workflow_rows.rs");

include!("workflow_restart.rs");

include!("workflow_status_detail.rs");

include!("workflow_cli_helpers.rs");

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
