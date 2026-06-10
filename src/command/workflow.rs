use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use archon_workflow::{
    CommandAction, ExecutionReport, HeuristicWorkflowPlanner, LifecycleAction, LifecycleController,
    RunStatus, TemplateRegistry, WorkflowCommand, WorkflowExecutor, WorkflowPlanner,
    WorkflowPolicy, WorkflowSpec, WorkflowStore,
};

use crate::cli_args::WorkflowAction;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::workflow_live::{run_live_cli_action, should_spawn_live, spawn_live_workflow};
use crate::command::workflow_world_learning;

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
            live,
            task,
        } => {
            let action = run_cli_action(spec_file.as_ref(), from_template.as_ref(), task)?;
            return Ok((action, mode(*live)));
        }
        WorkflowAction::Status { run_id } => CommandAction::Status {
            run_id: run_id.clone(),
        },
        WorkflowAction::Resume { live, run_id } => {
            return Ok((
                CommandAction::Resume {
                    run_id: run_id.clone(),
                },
                mode(*live),
            ));
        }
        WorkflowAction::Pause { run_id } => CommandAction::Pause {
            run_id: run_id.clone(),
        },
        WorkflowAction::Cancel { run_id } => CommandAction::Cancel {
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
        CommandAction::Run { task } => {
            let spec = planner.plan(&task)?;
            let report = execute_spec(&store, spec)?;
            deterministic_text(
                "Workflow complete",
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::RunSpec { path } => {
            let spec = load_spec_file(cwd, &path)?;
            let report = execute_spec(&store, spec)?;
            deterministic_text(
                "Workflow complete",
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::RunTemplate { name } => {
            let spec = load_template_spec(cwd, &name)?;
            let report = execute_spec(&store, spec)?;
            deterministic_text(
                "Workflow complete",
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::Status { run_id } => status_text(&store.load_state(&run_id)?),
        CommandAction::Resume { run_id } => {
            let run = store.load_state(&run_id)?;
            let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
            let report = executor.execute(run)?;
            deterministic_text(
                "Workflow resumed",
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::Pause { run_id } => lifecycle(&store, &run_id, LifecycleAction::Pause)?,
        CommandAction::Cancel { run_id } => lifecycle(&store, &run_id, LifecycleAction::Cancel)?,
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

pub(crate) fn load_spec_file(cwd: &Path, path: &str) -> Result<WorkflowSpec> {
    let path = resolve_input_path(cwd, path);
    let raw = fs::read_to_string(&path)?;
    WorkflowSpec::from_yaml(&raw).map_err(Into::into)
}

pub(crate) fn load_template_spec(cwd: &Path, name: &str) -> Result<WorkflowSpec> {
    Ok(TemplateRegistry::project(cwd).load(name)?.spec)
}

fn execute_spec(store: &WorkflowStore, spec: WorkflowSpec) -> Result<ExecutionReport> {
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec)?;
    executor.execute(run).map_err(Into::into)
}

fn deterministic_text(label: &str, report: ExecutionReport, learning_note: String) -> String {
    format!(
        "{label} (deterministic CLI smoke mode; pass --live or use TUI /workflow for LLM-backed agents): {} (completed {}, failed {}, skipped {})",
        report.run_id, report.completed, report.failed, report.skipped
    ) + "\n"
        + learning_note.as_str()
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

fn mode(live: bool) -> CliExecutionMode {
    if live {
        CliExecutionMode::Live
    } else {
        CliExecutionMode::Deterministic
    }
}

fn run_cli_action(
    spec_file: Option<&PathBuf>,
    from_template: Option<&String>,
    task: &[String],
) -> Result<CommandAction> {
    let selected =
        spec_file.is_some() as u8 + from_template.is_some() as u8 + (!task.is_empty()) as u8;
    if selected > 1 {
        return Err(anyhow!(
            "use exactly one of task text, --spec-file, or --from-template"
        ));
    }
    if let Some(path) = spec_file {
        return Ok(CommandAction::RunSpec {
            path: path.display().to_string(),
        });
    }
    if let Some(name) = from_template {
        return Ok(CommandAction::RunTemplate { name: name.clone() });
    }
    Ok(CommandAction::Run {
        task: task_string(task)?,
    })
}

fn ensure_no_task(task: &[String], flag: &str) -> Result<()> {
    if task.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("{flag} cannot be combined with task text"))
    }
}

fn task_string(parts: &[String]) -> Result<String> {
    let task = parts.join(" ");
    if task.trim().is_empty() {
        return Err(anyhow!("workflow task is required"));
    }
    Ok(task)
}

fn resolve_input_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
