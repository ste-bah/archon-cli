use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use archon_workflow::{
    CommandAction, HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, RunStatus,
    StageStatus, TemplateRegistry, WorkflowApprovalStore, WorkflowBundleOrigin, WorkflowCommand,
    WorkflowCommandRegistry, WorkflowPlanner, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2TaskInvalidation,
};

use crate::cli_args::WorkflowAction;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::workflow_live::{run_live_cli_action, should_spawn_live, spawn_live_workflow};

pub(crate) struct WorkflowHandler;

impl CommandHandler for WorkflowHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let cwd = ctx
            .working_dir
            .clone()
            .ok_or_else(|| anyhow!("workflow command requires working directory context"))?;
        // Intercepted ahead of `WorkflowCommand::parse` for the same reason the
        // CLI path intercepts ahead of `cli_action`: `CommandAction` is
        // `archon-workflow`'s execution vocabulary and an advisory read-only
        // analysis does not belong in it. Both surfaces therefore route `lint`
        // around the crate rather than through it.
        if args.first().is_some_and(|first| first == "lint") {
            let output = lint_from_slash_args(&cwd, &args[1..])?;
            ctx.emit(TuiEvent::TextDelta(output));
            ctx.emit(TuiEvent::SlashCommandComplete);
            return Ok(());
        }
        let command = WorkflowCommand::parse(args)?;
        if should_spawn_live(&command.action)
            && let Some(llm) = ctx.llm_adapter.clone()
        {
            spawn_live_workflow(
                cwd,
                command.action,
                // The interactive surface hands out the session's pipeline
                // client; the live workflow only ever sees it through the port.
                crate::command::pipeline_workflow_llm::PipelineWorkflowLlmClient::arc(llm),
                // Same shape as the LLM client above: the interactive surface
                // owns the TUI channel, and the live workflow only ever sees it
                // through the port.
                crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink::arc(ctx.tui_tx.clone()),
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
        "Plan, run, resume, lint, and inspect dynamic workflows"
    }
}

/// `/workflow lint --tasks <DIR>` and friends, parsed by hand.
///
/// The slash surface hands over raw tokens rather than a clap-parsed struct, so
/// the three flags are read directly. An unrecognised token is an error naming
/// the accepted flags: silently ignoring it would produce a report of something
/// other than what was asked for, which for a lint is worse than no report.
pub(crate) fn lint_from_slash_args(cwd: &Path, args: &[String]) -> Result<String> {
    let mut tasks: Option<PathBuf> = None;
    let mut spec_file: Option<PathBuf> = None;
    let mut graph: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let value = args.get(index + 1).cloned();
        let missing = |flag: &str| anyhow!("workflow lint {flag} needs a value");
        match args[index].as_str() {
            "--tasks" => tasks = Some(PathBuf::from(value.ok_or_else(|| missing("--tasks"))?)),
            "--spec-file" => {
                spec_file = Some(PathBuf::from(value.ok_or_else(|| missing("--spec-file"))?));
            }
            "--graph" => graph = Some(value.ok_or_else(|| missing("--graph"))?),
            other => {
                return Err(anyhow!(
                    "workflow lint does not accept '{other}'; use --tasks <DIR>, --spec-file <PATH>, or --graph <ID>"
                ));
            }
        }
        index += 2;
    }
    let source = crate::command::topology_lint::LintSource::from_flags(
        tasks.as_deref(),
        spec_file.as_deref(),
        graph.as_deref(),
    )?;
    crate::command::topology_lint::run_lint(cwd, &source)
}

pub(crate) async fn handle_workflow_command(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    // Intercepted before conversion: `lint` has no `CommandAction` counterpart
    // and deliberately does not gain one. `CommandAction` is `archon-workflow`'s
    // *execution* vocabulary — every variant names something that runs, resumes,
    // or mutates a run — and an advisory read-only analysis is none of those.
    // Adding a variant would put a milestone 4 concept inside the thin
    // provider-neutral crate for no gain.
    if let WorkflowAction::SyncCapabilities { tasks, dry_run } = action {
        // Same disposition as lint: derived from the task files, reported to
        // stdout, and it touches nothing but the manifest it names.
        let tasks_root = if tasks.is_absolute() {
            tasks.clone()
        } else {
            cwd.join(tasks)
        };
        let sync =
            crate::command::workflow_capabilities::sync_capabilities(&cwd, &tasks_root, *dry_run)?;
        print!("{}", sync.render());
        return Ok(());
    }
    if let WorkflowAction::Lint {
        tasks,
        spec_file,
        graph,
    } = action
    {
        let source = crate::command::topology_lint::LintSource::from_flags(
            tasks.as_deref(),
            spec_file.as_deref(),
            graph.as_deref(),
        )?;
        println!(
            "{}",
            crate::command::topology_lint::run_lint(&cwd, &source)?
        );
        return Ok(());
    }
    let (action, mode) = cli_action(action)?;
    let output = match mode {
        CliExecutionMode::Deterministic => run_action(&cwd, action)?,
        CliExecutionMode::Live => {
            // The bin crate is where the port gets its concrete implementation:
            // this is the last layer that can still name `archon-pipeline`.
            let llm_factory =
                crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::new(
                    config, env_vars,
                );
            run_live_cli_action(&cwd, action, config, env_vars, &llm_factory).await?
        }
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
            decomposed: _,
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
            decomposed,
            live,
            yes,
            task,
        } => {
            require_live_approval(*live, *yes, "workflow run --live")?;
            if let Some(run_id) = resume_from {
                ensure_resume_from_compatible(spec_file, from_template, *decomposed)?;
                return Ok((
                    CommandAction::Resume {
                        run_id: run_id.clone(),
                    },
                    mode(*live),
                ));
            }
            let action = run_cli_action(
                spec_file.as_ref(),
                from_template.as_ref(),
                task,
                *decomposed,
            )?;
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
        // Handled in `handle_workflow_command` before conversion; see the note
        // there on why it has no `CommandAction`.
        WorkflowAction::Lint { .. } => {
            return Err(anyhow!(
                "workflow lint is handled before action conversion and must not reach it"
            ));
        }
        WorkflowAction::SyncCapabilities { .. } => {
            return Err(anyhow!(
                "workflow sync-capabilities is handled before action conversion and must \
                 not reach it"
            ));
        }
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

#[path = "workflow_spec_execution.rs"]
mod workflow_spec_execution;
pub(crate) use workflow_spec_execution::*;

#[path = "workflow_rows.rs"]
mod workflow_rows;
use workflow_rows::*;

#[path = "workflow_restart.rs"]
mod workflow_restart;
use workflow_restart::*;

#[path = "workflow_status_detail.rs"]
mod workflow_status_detail;
use workflow_status_detail::*;

#[path = "workflow_cli_helpers.rs"]
mod workflow_cli_helpers;
use workflow_cli_helpers::*;

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
