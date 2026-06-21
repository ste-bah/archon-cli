use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use archon_workflow::{
    CommandAction, ExecutionReport, HeuristicWorkflowPlanner, LifecycleAction, LifecycleController,
    RunStatus, StageStatus, TemplateRegistry, WorkflowApprovalStore, WorkflowBundle,
    WorkflowBundleOrigin, WorkflowCommand, WorkflowCommandRegistry, WorkflowExecutor,
    WorkflowPlanner, WorkflowPolicy, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2ResultStore,
};

use crate::cli_args::WorkflowAction;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::workflow_live::{run_live_cli_action, should_spawn_live, spawn_live_workflow};
use crate::command::workflow_status_blocks;
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
            live,
            yes,
            task,
        } => {
            require_live_approval(*live, *yes, "workflow run --live")?;
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
        CommandAction::Run { task } => {
            let spec = planner.plan(&task)?;
            let report = execute_spec(&store, spec)?;
            deterministic_text(
                "Workflow complete",
                &store,
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::RunSpec { path } => {
            let spec = load_spec_file(cwd, &path)?;
            let report = execute_imported_spec(&store, spec)?;
            deterministic_text(
                "Workflow complete",
                &store,
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::RunTemplate { name } => {
            let template = load_template(cwd, &name)?;
            let report = execute_template(&store, template)?;
            deterministic_text(
                "Workflow complete",
                &store,
                report.clone(),
                workflow_world_learning::record_report(&store, &report),
            )
        }
        CommandAction::Status { run_id } => status_detail_text(&store, &run_id)?,
        CommandAction::Resume { run_id } => resume_workflow(&store, &run_id)?,
        CommandAction::Continue { run_id } => resume_workflow(&store, &run_id)?,
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

pub(crate) fn load_spec_file(cwd: &Path, path: &str) -> Result<WorkflowSpec> {
    let path = resolve_input_path(cwd, path);
    let raw = fs::read_to_string(&path)?;
    WorkflowSpec::from_yaml(&raw).map_err(Into::into)
}

fn execute_spec(store: &WorkflowStore, spec: WorkflowSpec) -> Result<ExecutionReport> {
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec)?;
    executor.execute(run).map_err(Into::into)
}

fn execute_imported_spec(store: &WorkflowStore, spec: WorkflowSpec) -> Result<ExecutionReport> {
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start_imported_spec(spec)?;
    executor.execute(run).map_err(Into::into)
}

pub(crate) struct LoadedWorkflowTemplate {
    pub spec: WorkflowSpec,
    pub harness_source: Option<String>,
}

pub(crate) fn load_template(cwd: &Path, name: &str) -> Result<LoadedWorkflowTemplate> {
    if let Some(command) = WorkflowCommandRegistry::project(cwd).load(name)? {
        return Ok(LoadedWorkflowTemplate {
            spec: command.spec,
            harness_source: Some(command.harness_source),
        });
    }
    Ok(LoadedWorkflowTemplate {
        spec: TemplateRegistry::project(cwd).load(name)?.spec,
        harness_source: None,
    })
}

fn execute_template(
    store: &WorkflowStore,
    template: LoadedWorkflowTemplate,
) -> Result<ExecutionReport> {
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = match template.harness_source {
        Some(harness) => executor.start_with_harness(
            template.spec,
            &harness,
            WorkflowBundleOrigin::SavedCommand,
        )?,
        None => executor.start(template.spec)?,
    };
    executor.execute(run).map_err(Into::into)
}

fn resume_workflow(store: &WorkflowStore, run_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let report = executor.execute(run)?;
    Ok(deterministic_text(
        "Workflow resumed",
        store,
        report.clone(),
        workflow_world_learning::record_report(store, &report),
    ))
}

fn repair_workflow(store: &WorkflowStore, run_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let stage_id = first_repairable_stage(&run).ok_or_else(|| {
        anyhow!("workflow {run_id} has no failed or blocked stage to repair; use /workflow status {run_id}")
    })?;
    let status = lifecycle(
        store,
        run_id,
        LifecycleAction::RestartStage(stage_id.clone()),
    )?;
    Ok(format!(
        "Workflow repair prepared: restarted failed/blocked stage {stage_id}.\nNext: /workflow continue {run_id}\n{status}"
    ))
}

fn restart_task_workflow(store: &WorkflowStore, run_id: &str, task_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let stage_id = stage_id_for_task(&run, task_id)
        .ok_or_else(|| anyhow!("task '{task_id}' did not match any workflow stage in {run_id}"))?;
    let status = lifecycle(
        store,
        run_id,
        LifecycleAction::RestartStage(stage_id.clone()),
    )?;
    Ok(format!(
        "Workflow task restart prepared: task {task_id} mapped to stage {stage_id}.\nNext: /workflow continue {run_id}\n{status}"
    ))
}

fn first_repairable_stage(run: &WorkflowRun) -> Option<String> {
    run.spec
        .stages
        .iter()
        .find(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == StageStatus::Failed)
        })
        .or_else(|| {
            run.spec.stages.iter().find(|stage| {
                run.stages
                    .get(&stage.id)
                    .is_some_and(|state| state.status == StageStatus::Blocked)
            })
        })
        .map(|stage| stage.id.clone())
}

fn stage_id_for_task(run: &WorkflowRun, task_id: &str) -> Option<String> {
    let aliases = task_aliases(task_id);
    if aliases.is_empty() {
        return None;
    }
    run.spec
        .stages
        .iter()
        .find(|stage| {
            stage_matches_task(&stage.id, &aliases)
                || stage
                    .task
                    .as_deref()
                    .is_some_and(|task| stage_matches_task(task, &aliases))
                || stage_matches_task(&stage.input.to_string(), &aliases)
        })
        .map(|stage| stage.id.clone())
}

fn stage_matches_task(value: &str, aliases: &[String]) -> bool {
    let normalized = normalize_task_token(value);
    let compact = normalized.replace(' ', "");
    aliases.iter().any(|alias| {
        normalized.split_whitespace().any(|token| token == alias)
            || normalized.contains(alias)
            || compact.contains(&alias.replace(' ', ""))
    })
}

fn task_aliases(task_id: &str) -> Vec<String> {
    let normalized = normalize_task_token(task_id);
    let compact = normalized.replace(' ', "");
    let mut aliases = Vec::new();
    push_alias(&mut aliases, normalized);
    push_alias(&mut aliases, compact.clone());
    let digits = compact
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if !digits.is_empty() {
        push_alias(&mut aliases, digits.clone());
        push_alias(&mut aliases, format!("t{digits}"));
    }
    aliases
}

fn push_alias(aliases: &mut Vec<String>, alias: String) {
    if !alias.is_empty() && !aliases.contains(&alias) {
        aliases.push(alias);
    }
}

fn normalize_task_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn deterministic_text(
    label: &str,
    store: &WorkflowStore,
    report: ExecutionReport,
    learning_note: String,
) -> String {
    let evidence_blocks = workflow_status_blocks::evidence_blocks(store, &report.run_id);
    format!(
        "{label} (deterministic CLI smoke mode; pass --live or use TUI /workflow for LLM-backed agents): {} (completed {}, failed {}, skipped {})",
        report.run_id, report.completed, report.failed, report.skipped
    ) + "\n"
        + learning_note.as_str()
        + evidence_blocks.as_str()
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
    let blocked = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Blocked))
        .count();
    let failed = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Failed))
        .count();
    EvidenceRowPayload {
        id: run.id.clone(),
        title: run.spec.name.clone(),
        status: format!("{:?}", run.status).to_ascii_lowercase(),
        detail: format!(
            "{accepted}/{} accepted, {blocked} blocked, {failed} failed, current={}, next={}",
            run.stages.len(),
            visible_stage_summary(run),
            next_workflow_action(run)
        ),
    }
}

fn lifecycle(store: &WorkflowStore, run_id: &str, action: LifecycleAction) -> Result<String> {
    let controller = LifecycleController::new(store.clone());
    let v2_restart = generated_v2_restart_target(&action);
    let run = controller.apply(run_id, action)?;
    let invalidated = match v2_restart {
        Some(GeneratedV2RestartTarget::Call(call_id)) => {
            invalidate_generated_v2_call(store, &run, &call_id)?
        }
        Some(GeneratedV2RestartTarget::Item { call_id, item_id }) => {
            invalidate_generated_v2_item(store, &run, &call_id, &item_id)?
        }
        None => Vec::new(),
    };
    let mut output = status_text(&run);
    if !invalidated.is_empty() {
        output.push_str(&format!(
            "\nV2 resume cache invalidated for {} call(s): {}",
            invalidated.len(),
            invalidated.join(", ")
        ));
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeneratedV2RestartTarget {
    Call(String),
    Item { call_id: String, item_id: String },
}

fn generated_v2_restart_target(action: &LifecycleAction) -> Option<GeneratedV2RestartTarget> {
    match action {
        LifecycleAction::RestartStage(stage_id) => {
            Some(GeneratedV2RestartTarget::Call(stage_id.clone()))
        }
        LifecycleAction::RestartItem { stage_id, item_id } => {
            Some(GeneratedV2RestartTarget::Item {
                call_id: stage_id.clone(),
                item_id: item_id.clone(),
            })
        }
        _ => None,
    }
}

fn invalidate_generated_v2_call(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
) -> Result<Vec<String>> {
    invalidate_generated_v2_call_cache(store, run, call_id, true)
}

fn invalidate_generated_v2_call_cache(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
    clear_branch_outcomes: bool,
) -> Result<Vec<String>> {
    let manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest)
            if matches!(
                manifest.origin,
                WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
            ) =>
        {
            manifest
        }
        _ => return Ok(Vec::new()),
    };
    let _ = manifest;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let mut invalidated = v2_store.invalidate_call_and_dependents(&[], call_id)?;
    if clear_branch_outcomes {
        let deleted = v2_store.delete_branch_outcomes_for_call(call_id)?;
        if deleted > 0 {
            invalidated.push(format!("{call_id}:branches({deleted})"));
        }
    }
    Ok(invalidated)
}

fn invalidate_generated_v2_item(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
    item_id: &str,
) -> Result<Vec<String>> {
    let mut invalidated = invalidate_generated_v2_call_cache(store, run, call_id, false)?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    for candidate in v2_branch_item_candidates(call_id, item_id) {
        if v2_store.delete_branch_outcome(call_id, &candidate)? {
            invalidated.push(format!("{call_id}:{candidate}"));
        }
    }
    Ok(invalidated)
}

fn v2_branch_item_candidates(call_id: &str, item_id: &str) -> Vec<String> {
    let mut candidates = vec![item_id.to_string()];
    let prefixed = format!("{call_id}-{item_id}");
    if !item_id.starts_with(&format!("{call_id}-")) {
        candidates.push(prefixed);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

#[derive(Debug, Clone, Copy)]
enum ApprovalCommand {
    RunOnce,
    Always,
    Deny,
}

fn approval(
    store: &WorkflowStore,
    cwd: &Path,
    run_id: &str,
    command: ApprovalCommand,
) -> Result<String> {
    let run = store.load_state(run_id)?;
    let approvals = WorkflowApprovalStore::project(cwd);
    let record = match command {
        ApprovalCommand::RunOnce => {
            approvals.approve_run_once(cwd, store, &run, "workflow-command")?
        }
        ApprovalCommand::Always => {
            approvals.approve_always_for_project(cwd, store, &run, "workflow-command")?
        }
        ApprovalCommand::Deny => {
            let record = approvals.deny_run(cwd, store, &run, "workflow-command")?;
            let _ =
                LifecycleController::new(store.clone()).apply(run_id, LifecycleAction::Cancel)?;
            record
        }
    };
    let action = match &record.decision {
        archon_workflow::WorkflowApprovalDecision::RunOnce => "approved once",
        archon_workflow::WorkflowApprovalDecision::AlwaysForProject => "approved always",
        archon_workflow::WorkflowApprovalDecision::Denied => "denied",
    };
    Ok(format!(
        "Workflow {run_id} {action}: {} phases, max_agents={}, max_parallelism={}, raw_script={}",
        record.phase_count, record.max_agents, record.max_parallelism, record.raw_script_path
    ))
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
    let blocked = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Blocked))
        .count();
    let forced = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::ForcedAccepted))
        .count();
    let status = match run.status {
        RunStatus::Planned => "planned",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::NeedsReview => "needs_review",
        RunStatus::Blocked => "blocked",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Completed => "completed",
    };
    format!(
        "Workflow {}: {} ({accepted}/{} accepted, {blocked} blocked, {forced} forced, {failed} failed, current={}, next={})",
        run.id,
        status,
        run.stages.len(),
        visible_stage_summary(run),
        next_workflow_action(run)
    )
}

fn visible_stage_summary(run: &WorkflowRun) -> String {
    for status in [
        StageStatus::Running,
        StageStatus::NeedsReview,
        StageStatus::Failed,
        StageStatus::Blocked,
        StageStatus::Pending,
        StageStatus::Paused,
    ] {
        if let Some(stage) = run.spec.stages.iter().find(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == status)
        }) {
            let mut summary = stage.id.clone();
            if let Some(task) = &stage.task {
                summary.push_str(": ");
                summary.push_str(&one_line(task, 90));
            }
            if let Some(error) = run
                .stages
                .get(&stage.id)
                .and_then(|state| state.error.as_ref())
            {
                summary.push_str(" error=");
                summary.push_str(&one_line(error, 90));
            }
            return summary;
        }
    }
    "none".to_string()
}

fn next_workflow_action(run: &WorkflowRun) -> String {
    match run.status {
        RunStatus::NeedsReview => format!("/workflow resume --live {}", run.id),
        RunStatus::Failed | RunStatus::Blocked => format!("/workflow repair {}", run.id),
        RunStatus::Paused => format!("/workflow continue {}", run.id),
        RunStatus::Running | RunStatus::Planned => format!("wait or /workflow status {}", run.id),
        RunStatus::Completed => "review final report".to_string(),
        RunStatus::Cancelled => "start a new workflow".to_string(),
    }
}

fn status_detail_text(store: &WorkflowStore, run_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let mut out = status_text(&run);
    out.push('\n');
    out.push_str(&format!(
        "name: {}\ntask: {}\ncreated: {}\nupdated: {}\ngeneration: {}\n",
        run.spec.name, run.spec.task, run.created_at, run.updated_at, run.generation
    ));
    match archon_workflow::WorkflowBundle::verify(store, run_id) {
        Ok(manifest) => {
            out.push_str(&format!(
                "bundle: verified workflow.js={} workflow.compiled.yaml={} phases={} max_agents={} max_parallelism={} write_capable={}\n",
                short_hash(&manifest.workflow_hash),
                short_hash(&manifest.compiled_hash),
                manifest.phase_count,
                manifest.max_agents,
                manifest.max_parallelism,
                display_list(&manifest.write_capable_stages)
            ));
        }
        Err(err) => {
            out.push_str(&format!("bundle: not verified ({err})\n"));
        }
    }

    out.push_str("\nphases/stages:\n");
    for stage in &run.spec.stages {
        let state = run.stages.get(&stage.id);
        let status = state
            .map(|state| format!("{:?}", state.status).to_ascii_lowercase())
            .unwrap_or_else(|| "missing".to_string());
        let attempts = state.map_or(0, |state| state.attempt);
        let artifact_count = state.map_or(0, |state| state.artifacts.len());
        out.push_str(&format!(
            "- {} kind={:?} status={} attempts={} depends_on={} artifacts={}",
            stage.id,
            stage.kind,
            status,
            attempts,
            display_list(&stage.depends_on),
            artifact_count
        ));
        if let Some(tier) = stage.provider_tier {
            out.push_str(&format!(" tier={tier:?}"));
        }
        if let Some(item_kind) = stage.item_kind {
            out.push_str(&format!(" item_kind={item_kind:?}"));
        }
        if let Some(max_parallelism) = stage.max_parallelism {
            out.push_str(&format!(" max_parallelism={max_parallelism}"));
        }
        if !stage.expected_target_files.is_empty() {
            out.push_str(&format!(
                " target_files={}",
                display_list(&stage.expected_target_files)
            ));
        }
        if let Some(command) = &stage.verify_command {
            out.push_str(&format!(" verify_command={}", one_line(command, 140)));
        }
        if let Some(state) = state {
            if let Some(started_at) = state.started_at {
                out.push_str(&format!(" started={started_at}"));
            }
            if let Some(completed_at) = state.completed_at {
                out.push_str(&format!(" completed={completed_at}"));
            }
            if let Some(error) = &state.error {
                out.push_str(&format!(" error={}", one_line(error, 180)));
            }
        }
        out.push('\n');
    }

    if !run.items.is_empty() {
        out.push_str("\nitems:\n");
        for item in run.items.values() {
            out.push_str(&format!(
                "- {} stage={} status={:?}",
                item.id, item.stage_id, item.status
            ));
            if let Some(artifact) = &item.artifact {
                out.push_str(&format!(" artifact={}", artifact.id));
            }
            if let Some(error) = &item.error {
                out.push_str(&format!(" error={}", one_line(error, 160)));
            }
            out.push('\n');
        }
    }

    let prompts = prompt_previews(store, run_id)?;
    if !prompts.is_empty() {
        out.push_str("\nprompts:\n");
        for prompt in prompts {
            out.push_str(&format!(
                "- stage={} item={} input_hash={} prompt_hash={} created={} path={}\n",
                prompt.stage_id,
                prompt.item_id,
                short_hash(&prompt.input_hash),
                short_hash(&prompt.prompt_hash),
                prompt.created_at.unwrap_or_else(|| "unknown".to_string()),
                prompt.path.display()
            ));
        }
    }

    if let Ok(detail) = archon_workflow::web_api::detail(store, run_id) {
        if !detail.agents.is_empty() {
            out.push_str("\nagent outputs:\n");
            for agent in detail.agents {
                out.push_str(&format!(
                    "- stage={} item={} status={} provider={} model={} tokens={}/{} cost=${:.6} output={}",
                    agent.stage_id,
                    agent.item_id,
                    agent.status,
                    agent.provider.unwrap_or_else(|| "unknown".to_string()),
                    agent.model.unwrap_or_else(|| "unknown".to_string()),
                    agent.tokens_in,
                    agent.tokens_out,
                    agent.cost_usd,
                    agent.output_path
                ));
                if let Some(artifact_id) = agent.artifact_id {
                    out.push_str(&format!(" artifact={artifact_id}"));
                }
                if let Some(error) = agent.error {
                    out.push_str(&format!(" error={}", one_line(&error, 160)));
                }
                if let Some(preview) = agent.result_preview {
                    out.push_str(&format!(" preview={}", one_line(&preview, 180)));
                }
                out.push('\n');
            }
        }
        if !detail.artifacts.is_empty() {
            out.push_str("\nartifacts:\n");
            for artifact in detail.artifacts {
                out.push_str(&format!(
                    "- {} stage={} path={} hash={}\n",
                    artifact.id,
                    artifact.producing_stage,
                    artifact.path,
                    short_hash(&artifact.content_hash)
                ));
            }
        }
        if !detail.events.is_empty() {
            out.push_str("\nrecent events:\n");
            for event in detail.events.into_iter().take(8) {
                out.push_str(&format!(
                    "- #{} {} {} {}\n",
                    event.seq,
                    event.created_at,
                    event.status,
                    one_line(&event.summary, 120)
                ));
            }
        }
    }
    Ok(out)
}

#[derive(Debug)]
struct PromptPreview {
    stage_id: String,
    item_id: String,
    input_hash: String,
    prompt_hash: String,
    created_at: Option<String>,
    path: PathBuf,
}

fn prompt_previews(store: &WorkflowStore, run_id: &str) -> Result<Vec<PromptPreview>> {
    let root = store.run_dir(run_id).join("prompts");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect_json_files(&root, &mut paths)?;
    let mut previews = Vec::new();
    for path in paths {
        let raw = fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        previews.push(PromptPreview {
            stage_id: string_json_field(&value, "stage_id").unwrap_or_default(),
            item_id: string_json_field(&value, "item_id").unwrap_or_default(),
            input_hash: string_json_field(&value, "input_hash").unwrap_or_default(),
            prompt_hash: string_json_field(&value, "prompt_hash").unwrap_or_default(),
            created_at: string_json_field(&value, "created_at"),
            path: path
                .strip_prefix(store.run_dir(run_id))
                .unwrap_or(path.as_path())
                .to_path_buf(),
        });
    }
    previews.sort_by(|a, b| {
        a.stage_id
            .cmp(&b.stage_id)
            .then_with(|| a.item_id.cmp(&b.item_id))
    });
    Ok(previews)
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(())
}

fn string_json_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn display_list(values: &[String]) -> String {
    if values.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", values.join(", "))
    }
}

fn one_line(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let preview = compact.chars().take(max_chars).collect::<String>();
        format!("{preview}...")
    }
}

fn short_hash(value: &str) -> String {
    value.chars().take(12).collect()
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

fn require_live_approval(live: bool, yes: bool, command: &str) -> Result<()> {
    if live && !yes {
        return Err(anyhow!(
            "{command} requires --yes in non-interactive CLI mode so the generated workflow is explicitly approved"
        ));
    }
    Ok(())
}

fn resolve_input_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GeneratedV2RestartTarget, WorkflowHandler, generated_v2_restart_target,
        invalidate_generated_v2_call, invalidate_generated_v2_item, restart_task_workflow,
        stage_id_for_task, status_text,
    };
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::{CtxBuilder, drain_tui_events};
    use archon_tui::app::TuiEvent;
    use archon_workflow::{
        ProviderTier, RetryPolicy, RunStatus, StageKind, StageSpec, StageStatus, WorkflowBundle,
        WorkflowBundleOrigin, WorkflowRun, WorkflowSpec, WorkflowStore, WorkflowV2BranchOutcome,
        WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2ResultStore,
        WorkflowV2Status,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn workflow_list_completes_tui_slash_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(temp.path().to_path_buf())
            .build();

        WorkflowHandler
            .execute(&mut ctx, &[String::from("list")])
            .unwrap();

        let events = drain_tui_events(&mut rx);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TuiEvent::OpenViewRows { .. })),
            "workflow list should emit workflow rows"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TuiEvent::SlashCommandComplete)),
            "workflow command must complete the slash lifecycle"
        );
    }

    #[test]
    fn restart_task_resolver_matches_canonical_task_variants() {
        let run = WorkflowRun::new(test_spec(), tempfile::tempdir().unwrap().path());

        assert_eq!(
            stage_id_for_task(&run, "T010").as_deref(),
            Some("implement-T010-T020")
        );
        assert_eq!(
            stage_id_for_task(&run, "TASK-GEN-010").as_deref(),
            Some("implement-T010-T020")
        );
    }

    #[test]
    fn restart_task_command_rewinds_matching_stage() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let mut run = store.create_run(test_spec()).unwrap();
        run.status = RunStatus::Failed;
        run.stages.get_mut("implement-T010-T020").unwrap().status = StageStatus::Failed;
        run.stages.get_mut("implement-T010-T020").unwrap().error = Some("boom".to_string());
        store.save_state(&run).unwrap();

        let output = restart_task_workflow(&store, &run.id, "TASK-GEN-010").unwrap();
        let reloaded = store.load_state(&run.id).unwrap();

        assert!(output.contains("task TASK-GEN-010 mapped to stage implement-T010-T020"));
        assert_eq!(reloaded.status, RunStatus::Running);
        assert_eq!(
            reloaded.stages.get("implement-T010-T020").unwrap().status,
            StageStatus::Pending
        );
        assert!(
            reloaded
                .stages
                .get("implement-T010-T020")
                .unwrap()
                .error
                .is_none()
        );
    }

    #[test]
    fn status_summary_reports_current_stage_and_next_action() {
        let mut run = WorkflowRun::new(test_spec(), tempfile::tempdir().unwrap().path());
        run.status = RunStatus::Failed;
        run.stages.get_mut("implement-T010-T020").unwrap().status = StageStatus::Failed;
        run.stages.get_mut("implement-T010-T020").unwrap().error =
            Some("verification failed".to_string());

        let summary = status_text(&run);

        assert!(summary.contains("current=implement-T010-T020"));
        assert!(summary.contains("error=verification failed"));
        assert!(summary.contains(&format!("next=/workflow repair {}", run.id)));
    }

    #[test]
    fn generated_v2_restart_item_invalidates_parent_fanout_call() {
        let action = archon_workflow::LifecycleAction::RestartItem {
            stage_id: "implementation-fanout".to_string(),
            item_id: "implementation-fanout-T010".to_string(),
        };

        assert_eq!(
            generated_v2_restart_target(&action),
            Some(GeneratedV2RestartTarget::Item {
                call_id: "implementation-fanout".to_string(),
                item_id: "implementation-fanout-T010".to_string(),
            })
        );
    }

    #[test]
    fn generated_v2_restart_item_deletes_only_requested_branch_cache() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store.create_run(test_spec()).unwrap();
        let harness = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed items." });
  const implementation = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Implement one item." });
  await w.finalReport("final", { inputs: [inventory, implementation], task: "Report evidence." });
}
"#;
        WorkflowBundle::create_for_run(
            &store,
            &run,
            harness,
            WorkflowBundleOrigin::GeneratedHarness,
        )
        .unwrap();
        let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        save_test_branch(&v2_store, "implementation", "implementation-T001");
        save_test_branch(&v2_store, "implementation", "implementation-T002");

        let invalidated =
            invalidate_generated_v2_item(&store, &run, "implementation", "T001").unwrap();

        assert!(invalidated.iter().any(|id| id == "implementation"));
        assert!(
            v2_store
                .load_branch_outcome("implementation", "implementation-T001")
                .unwrap()
                .is_none()
        );
        assert!(
            v2_store
                .load_branch_outcome("implementation", "implementation-T002")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn generated_v2_restart_stage_deletes_all_branch_cache_for_call() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store.create_run(test_spec()).unwrap();
        let harness = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed items." });
  const implementation = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Implement one item." });
  await w.finalReport("final", { inputs: [inventory, implementation], task: "Report evidence." });
}
"#;
        WorkflowBundle::create_for_run(
            &store,
            &run,
            harness,
            WorkflowBundleOrigin::GeneratedHarness,
        )
        .unwrap();
        let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        save_test_branch(&v2_store, "implementation", "implementation-T001");
        save_test_branch(&v2_store, "implementation", "implementation-T002");

        let invalidated = invalidate_generated_v2_call(&store, &run, "implementation").unwrap();

        assert!(invalidated.iter().any(|id| id == "implementation"));
        assert!(
            invalidated
                .iter()
                .any(|id| id == "implementation:branches(2)")
        );
        assert!(
            v2_store
                .load_branch_outcome("implementation", "implementation-T001")
                .unwrap()
                .is_none()
        );
        assert!(
            v2_store
                .load_branch_outcome("implementation", "implementation-T002")
                .unwrap()
                .is_none()
        );
    }

    fn test_spec() -> WorkflowSpec {
        WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "test".to_string(),
            task: "Implement a decomposed PRD".to_string(),
            target_repository_root: None,
            max_parallelism: 4,
            max_agents: 16,
            provider_tiers: BTreeMap::from([(ProviderTier::Coder, "test".to_string())]),
            stages: vec![
                test_stage(
                    "read-only-review",
                    "Read the PRD and inspect current implementation.",
                    json!({"task_ids": ["TASK-GEN-001"]}),
                    vec![],
                ),
                test_stage(
                    "implement-T010-T020",
                    "Implement T010 and T020.",
                    json!({"task_ids": ["TASK-GEN-010", "TASK-GEN-020"]}),
                    vec!["read-only-review".to_string()],
                ),
            ],
            artifact_policy: Default::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }

    fn test_stage(
        id: &str,
        task: &str,
        input: serde_json::Value,
        depends_on: Vec<String>,
    ) -> StageSpec {
        StageSpec {
            id: id.to_string(),
            kind: StageKind::Agent,
            task: Some(task.to_string()),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            condition: None,
            depends_on,
            provider_tier: Some(ProviderTier::Coder),
            retry: RetryPolicy::default(),
            input,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: BTreeMap::new(),
        }
    }

    fn save_test_branch(v2_store: &WorkflowV2ResultStore, call_id: &str, item_id: &str) {
        let mut result = WorkflowV2Result::accepted(format!("branch {item_id} accepted"));
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            format!("branch {item_id} has concrete cached evidence"),
        ));
        v2_store
            .save_branch_outcome(
                call_id,
                &WorkflowV2BranchOutcome {
                    item_id: item_id.to_string(),
                    role: "coder".to_string(),
                    status: WorkflowV2Status::Accepted,
                    result: Some(result),
                    error: None,
                },
            )
            .unwrap();
    }
}
