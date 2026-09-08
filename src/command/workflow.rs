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
#[path = "workflow_cli_lint.rs"]
mod workflow_cli_lint;
#[path = "workflow_decompose_cli.rs"]
mod workflow_decompose_cli;
#[path = "workflow_freeze_cli.rs"]
mod workflow_freeze_cli;
#[path = "workflow_staged_cli.rs"]
mod workflow_staged_cli;
pub(crate) use workflow_cli_lint::lint_from_slash_args;

pub(crate) struct WorkflowHandler;

impl CommandHandler for WorkflowHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let cwd = ctx
            .working_dir
            .clone()
            .ok_or_else(|| anyhow!("workflow command requires working directory context"))?;
        if crate::command::fixed_decomposition_host::handle_command_context(ctx, args, cwd.clone())?
        {
            return Ok(());
        }
        // Intercepted ahead of `WorkflowCommand::parse` for the same reason the
        // CLI path intercepts ahead of `cli_action`: `CommandAction` is
        // `archon-workflow`'s execution vocabulary and an advisory read-only
        // analysis does not belong in it. Both surfaces therefore route `lint`
        // around the crate rather than through it.
        if args
            .first()
            .is_some_and(|first| matches!(first.as_str(), "freeze-acceptance" | "freeze-skeleton"))
        {
            return Err(anyhow!(
                "/workflow {} is CLI-only; run `archon workflow {} --tasks <DIR> --prd <PATH>` in a terminal",
                args[0],
                args[0]
            ));
        }
        if args.first().is_some_and(|first| first == "lint") {
            let source = workflow_cli_lint::lint_source_from_slash_args(&args[1..])?;
            let mode = ctx.gate_mode.unwrap_or_default();
            let gate_id = match source {
                crate::command::topology_lint::LintSource::TaskFile(_) => {
                    crate::command::workflow_gate::GateId::WorkflowLintTaskFile
                }
                _ => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
            };
            let disposition =
                crate::command::workflow_gate::run_sync_gate(&cwd, mode, gate_id, || {
                    crate::command::topology_lint::evaluate_lint(&cwd, &source, mode)
                })?;
            ctx.emit(TuiEvent::TextDelta(disposition.report().to_string()));
            for diagnostic in disposition.diagnostics() {
                ctx.emit(TuiEvent::TextDelta(format!("{diagnostic}\n")));
            }
            disposition.require_allowed()?;
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
                crate::command::pipeline_workflow_llm::PipelineWorkflowLlmClient::configured(
                    llm, ctx.workflow_config.as_ref().ok_or_else(||anyhow!("workflow requires resolved operator configuration"))?,
                ),
                // Same shape as the LLM client above: the interactive surface
                // owns the TUI channel, and the live workflow only ever sees it
                // through the port.
                // Resilient: a closed TUI channel degrades the run to quiet
                // instead of failing whichever branch was emitting.
                archon_workflow::ui_sink_port::ResilientWorkflowUiSink::wrap(
                    crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink::arc(
                        ctx.tui_tx.clone(),
                    ),
                ),
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
    if workflow_decompose_cli::handle(action, config, env_vars, &cwd).await? {
        return Ok(());
    }
    if workflow_freeze_cli::handle(action, config, env_vars, &cwd).await? {
        return Ok(());
    }
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
        task_file,
        tasks,
        spec_file,
        graph,
        candidate_stdin,
        staging_root,
        gate_envelope,
        call_id,
    } = action
    {
        if *candidate_stdin || staging_root.is_some() {
            workflow_staged_cli::handle_staged_task_file_lint(
                &cwd,
                task_file.as_deref(),
                tasks.as_deref(),
                spec_file.as_deref(),
                graph.as_deref(),
                *candidate_stdin,
                staging_root.as_deref(),
                gate_envelope.as_deref(),
                call_id.as_deref(),
                config.workflow.gate_mode,
            )?;
            return Ok(());
        }
        if gate_envelope.is_some() || call_id.is_some() {
            workflow_staged_cli::handle_staged_task_set_lint(
                &cwd,
                task_file.as_deref(),
                tasks.as_deref(),
                spec_file.as_deref(),
                graph.as_deref(),
                gate_envelope.as_deref(),
                call_id.as_deref(),
                config.workflow.gate_mode,
            )?;
            return Ok(());
        }
        let source = crate::command::topology_lint::LintSource::from_flags(
            task_file.as_deref(),
            tasks.as_deref(),
            spec_file.as_deref(),
            graph.as_deref(),
        )?;
        let gate_id = match source {
            crate::command::topology_lint::LintSource::TaskFile(_) => {
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile
            }
            _ => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        };
        let disposition = crate::command::workflow_gate::run_sync_gate(
            &cwd,
            config.workflow.gate_mode,
            gate_id,
            || {
                crate::command::topology_lint::evaluate_lint(
                    &cwd,
                    &source,
                    config.workflow.gate_mode,
                )
            },
        )?;
        print!("{}", disposition.report());
        for diagnostic in disposition.diagnostics() {
            eprintln!("{diagnostic}");
        }
        disposition.require_allowed()?;
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
        WorkflowAction::FreezeAcceptance { .. } | WorkflowAction::FreezeSkeleton { .. } => {
            return Err(anyhow!(
                "workflow freeze action is handled before action conversion and must not reach it"
            ));
        }
        WorkflowAction::SyncCapabilities { .. } => {
            return Err(anyhow!(
                "workflow sync-capabilities is handled before action conversion and must \
                 not reach it"
            ));
        }
        WorkflowAction::Decompose { .. } | WorkflowAction::DecompositionIdentity => {
            return Err(anyhow!(
                "fixed decomposition action is handled before action conversion and must not reach it"
            ));
        }
    };
    Ok((converted, CliExecutionMode::Deterministic))
}

#[path = "workflow_run_action.rs"]
mod workflow_run_action;
pub(crate) use workflow_run_action::{run_action, run_action_authorized};

#[path = "workflow_spec_execution.rs"]
mod workflow_spec_execution;
pub(crate) use workflow_spec_execution::*;

#[path = "workflow_rows.rs"]
mod workflow_rows;
use workflow_rows::*;

#[path = "workflow_restart.rs"]
mod workflow_restart;
use workflow_restart::*;

#[path = "workflow_finalization_status.rs"]
mod workflow_finalization_status;
#[path = "workflow_status_detail.rs"]
mod workflow_status_detail;
use workflow_status_detail::*;

#[path = "workflow_cli_helpers.rs"]
mod workflow_cli_helpers;
use workflow_cli_helpers::*;

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
