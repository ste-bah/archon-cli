use std::path::Path;

use anyhow::{Result, anyhow};
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use archon_workflow::{
    CommandAction, HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, RunStatus,
    TemplateRegistry, WorkflowCommand, WorkflowExecutor, WorkflowPlanner, WorkflowPolicy,
    WorkflowStore,
};

use crate::cli_args::WorkflowAction;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::workflow_live::{should_spawn_live, spawn_live_workflow};

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
            spawn_live_workflow(cwd, command.action, llm, ctx.tui_tx.clone());
            return Ok(());
        }
        if matches!(
            command.action,
            CommandAction::List | CommandAction::Status { .. }
        ) && emit_workflow_rows(&cwd, &command.action, ctx)?
        {
            return Ok(());
        }
        let output = run_action(&cwd, command.action)?;
        ctx.emit(TuiEvent::TextDelta(output));
        Ok(())
    }

    fn description(&self) -> &str {
        "Plan, run, resume, and inspect dynamic workflows"
    }
}

pub(crate) fn handle_workflow_command(action: &WorkflowAction) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let output = run_action(&cwd, cli_action(action)?)?;
    println!("{output}");
    Ok(())
}

fn cli_action(action: &WorkflowAction) -> Result<CommandAction> {
    let converted = match action {
        WorkflowAction::Plan { task } => CommandAction::Plan {
            task: task_string(task)?,
        },
        WorkflowAction::Run { task } => CommandAction::Run {
            task: task_string(task)?,
        },
        WorkflowAction::Status { run_id } => CommandAction::Status {
            run_id: run_id.clone(),
        },
        WorkflowAction::Resume { run_id } => CommandAction::Resume {
            run_id: run_id.clone(),
        },
        WorkflowAction::Pause { run_id } => CommandAction::Pause {
            run_id: run_id.clone(),
        },
        WorkflowAction::Cancel { run_id } => CommandAction::Cancel {
            run_id: run_id.clone(),
        },
        WorkflowAction::RestartAgent { run_id, stage_id } => CommandAction::RestartAgent {
            run_id: run_id.clone(),
            stage_id: stage_id.clone(),
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
    Ok(converted)
}

pub(super) fn run_action(cwd: &Path, action: CommandAction) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let planner = HeuristicWorkflowPlanner;
    let text = match action {
        CommandAction::Plan { task } => planner.plan(&task)?.to_yaml()?,
        CommandAction::Run { task } => {
            let spec = planner.plan(&task)?;
            let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
            let run = executor.start(spec)?;
            let report = executor.execute(run)?;
            format!(
                "Workflow complete: {} (completed {}, failed {}, skipped {})",
                report.run_id, report.completed, report.failed, report.skipped
            )
        }
        CommandAction::Status { run_id } => status_text(&store.load_state(&run_id)?),
        CommandAction::Resume { run_id } => {
            let run = store.load_state(&run_id)?;
            let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
            let report = executor.execute(run)?;
            format!(
                "Workflow resumed: {} (completed {}, failed {}, skipped {})",
                report.run_id, report.completed, report.failed, report.skipped
            )
        }
        CommandAction::Pause { run_id } => lifecycle(&store, &run_id, LifecycleAction::Pause)?,
        CommandAction::Cancel { run_id } => lifecycle(&store, &run_id, LifecycleAction::Cancel)?,
        CommandAction::RestartAgent { run_id, stage_id } => {
            lifecycle(&store, &run_id, LifecycleAction::RestartStage(stage_id))?
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
            let template = TemplateRegistry::project(cwd).save(&name, &run.spec)?;
            format!("Workflow template saved: {}", template.name)
        }
        CommandAction::List => list_text(&store)?,
    };
    Ok(text)
}

fn emit_workflow_rows(
    cwd: &Path,
    action: &CommandAction,
    ctx: &mut CommandContext,
) -> Result<bool> {
    let store = WorkflowStore::project(cwd);
    let rows = match action {
        CommandAction::List => store
            .list_runs()?
            .iter()
            .map(run_row)
            .collect::<Vec<EvidenceRowPayload>>(),
        CommandAction::Status { run_id } => {
            let run = store.load_state(run_id)?;
            run.stages
                .values()
                .map(|stage| EvidenceRowPayload {
                    id: stage.id.clone(),
                    title: stage.id.clone(),
                    status: format!("{:?}", stage.status).to_ascii_lowercase(),
                    detail: format!(
                        "attempts={} artifacts={}{}",
                        stage.attempt,
                        stage.artifacts.len(),
                        stage
                            .error
                            .as_ref()
                            .map(|error| format!(" error={error}"))
                            .unwrap_or_default()
                    ),
                })
                .collect()
        }
        _ => return Ok(false),
    };
    ctx.emit(TuiEvent::OpenViewRows {
        view_id: ViewId::Workflow,
        rows,
    });
    Ok(true)
}

fn run_row(run: &archon_workflow::WorkflowRun) -> EvidenceRowPayload {
    let accepted = run
        .stages
        .values()
        .filter(|stage| run.accepted_stage(&stage.id))
        .count();
    EvidenceRowPayload {
        id: run.id.clone(),
        title: run.spec.name.clone(),
        status: format!("{:?}", run.status).to_ascii_lowercase(),
        detail: format!("{accepted}/{} accepted", run.stages.len()),
    }
}

fn lifecycle(store: &WorkflowStore, run_id: &str, action: LifecycleAction) -> Result<String> {
    let controller = LifecycleController::new(store.clone());
    let run = controller.apply(run_id, action)?;
    Ok(status_text(&run))
}

fn status_text(run: &archon_workflow::WorkflowRun) -> String {
    let accepted = run
        .stages
        .values()
        .filter(|stage| run.accepted_stage(&stage.id))
        .count();
    let failed = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Failed))
        .count();
    let status = match run.status {
        RunStatus::Planned => "planned",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Completed => "completed",
    };
    format!(
        "Workflow {}: {} ({accepted}/{} accepted, {failed} failed)",
        run.id,
        status,
        run.stages.len()
    )
}

fn list_text(store: &WorkflowStore) -> Result<String> {
    let runs = store.list_runs()?;
    if runs.is_empty() {
        return Ok("No workflow runs found.".to_string());
    }
    Ok(runs.iter().map(status_text).collect::<Vec<_>>().join("\n"))
}

fn task_string(parts: &[String]) -> Result<String> {
    let task = parts.join(" ");
    if task.trim().is_empty() {
        return Err(anyhow!("workflow task is required"));
    }
    Ok(task)
}
