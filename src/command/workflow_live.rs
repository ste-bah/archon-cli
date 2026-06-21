use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_core::agents::AgentRegistry;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    CommandAction, LifecycleAction, LifecycleController, RunStatus, StageStatus,
    WorkflowBundleOrigin, WorkflowConfig, WorkflowExecutor, WorkflowPolicy, WorkflowRun,
    WorkflowStageRunner, WorkflowStore, WorkflowV2HarnessValidator,
};

use crate::command::pipeline_support::build_pipeline_adapter;
use crate::command::workflow::{load_spec_file, load_template, run_action};
use crate::command::workflow_status_blocks;
use crate::command::workflow_world_learning;

#[cfg(test)]
#[path = "workflow_live_planner_repair_tests.rs"]
mod planner_repair_tests;
#[cfg(test)]
#[path = "workflow_live_tests.rs"]
mod tests;
#[path = "workflow_agent_select.rs"]
mod workflow_agent_select;
#[path = "workflow_live_approval.rs"]
mod workflow_live_approval;
#[path = "workflow_live_compat.rs"]
mod workflow_live_compat;
#[cfg(test)]
#[path = "workflow_live_execution_tests.rs"]
mod workflow_live_execution_tests;
#[path = "workflow_live_items.rs"]
mod workflow_live_items;
#[path = "workflow_live_planner.rs"]
mod workflow_live_planner;
#[path = "workflow_live_prompt.rs"]
mod workflow_live_prompt;
#[path = "workflow_live_retry.rs"]
mod workflow_live_retry;
#[path = "workflow_live_runner.rs"]
mod workflow_live_runner;
#[cfg(test)]
#[path = "workflow_live_runner_tests.rs"]
mod workflow_live_runner_tests;
#[cfg(test)]
#[path = "workflow_live_test_support.rs"]
mod workflow_live_test_support;
#[path = "workflow_live_v2.rs"]
mod workflow_live_v2;
#[path = "workflow_live_v2_host.rs"]
mod workflow_live_v2_host;
#[cfg(test)]
#[path = "workflow_live_v2_host_tests.rs"]
mod workflow_live_v2_host_tests;
#[cfg(test)]
#[path = "workflow_v2_live_tests.rs"]
mod workflow_v2_live_tests;

use workflow_live_approval::{LiveApprovalOutcome, gate_live_approval};
use workflow_live_planner::{LivePlan, plan_live, render_live_plan};
use workflow_live_runner::PipelineWorkflowRunner;

pub(crate) fn should_spawn_live(action: &CommandAction) -> bool {
    matches!(
        action,
        CommandAction::Plan { .. }
            | CommandAction::Run { .. }
            | CommandAction::RunSpec { .. }
            | CommandAction::RunTemplate { .. }
            | CommandAction::Resume { .. }
            | CommandAction::Continue { .. }
    )
}

pub(crate) fn spawn_live_workflow(
    cwd: PathBuf,
    action: CommandAction,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    config_path: Option<PathBuf>,
) {
    let _ = tui_tx.send(TuiEvent::TextDelta(live_start_message(&action)));
    archon_observability::spawn_named("dynamic-workflow-run", async move {
        let result = run_live_action(
            &cwd,
            action,
            llm,
            tui_tx.clone(),
            config_path,
            true,
            LiveApprovalMode::InteractiveSurface,
        )
        .await;
        match result {
            Ok(text) => {
                let _ = tui_tx.send(TuiEvent::TextDelta(text));
            }
            Err(err) => {
                let message = format!("Workflow failed: {err}");
                let _ = tui_tx.send(TuiEvent::TextDelta(format!("{message}\n")));
                let _ = tui_tx.send(TuiEvent::Error(message));
            }
        }
    });
}

pub(crate) async fn run_live_cli_action(
    cwd: &Path,
    action: CommandAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<String> {
    let adapter = build_pipeline_adapter(config, env_vars, "workflow_cli").await?;
    let llm: Arc<dyn LlmClient> = Arc::new(adapter);
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(128);
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(archon_core::config::default_config_path);
    run_live_action(
        cwd,
        action,
        llm,
        tui_tx,
        Some(config_path),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveApprovalMode {
    CliYes,
    InteractiveSurface,
}

impl LiveApprovalMode {
    fn decided_by(self) -> &'static str {
        match self {
            Self::CliYes => "cli --yes",
            Self::InteractiveSurface => "interactive workflow surface",
        }
    }
}

async fn run_live_action(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    config_path: Option<PathBuf>,
    workspace_boundary_supported: bool,
    approval_mode: LiveApprovalMode,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let executor = WorkflowExecutor::new(store.clone(), live_policy(cwd, config_path.as_deref()));
    let runner = PipelineWorkflowRunner {
        llm: llm.clone(),
        tui_tx: tui_tx.clone(),
        agent_names: AgentRegistry::load(cwd)
            .available_agent_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
        workspace_boundary_supported,
    };
    let mut approval_notes = Vec::new();
    let report = match action {
        CommandAction::Plan { task } => {
            let mut plan = plan_live(&store, &task, llm.clone(), tui_tx.clone()).await?;
            cap_live_plan_parallelism(&mut plan, &runner);
            return Ok(render_live_plan(&plan)?);
        }
        CommandAction::PlanSpec { path } => return Ok(load_spec_file(cwd, &path)?.to_yaml()?),
        CommandAction::Run { task } => {
            let mut plan = plan_live(&store, &task, llm.clone(), tui_tx.clone()).await?;
            cap_live_plan_parallelism(&mut plan, &runner);
            return workflow_live_v2::run_generated_v2_workflow(
                cwd,
                &store,
                plan,
                task,
                llm,
                tui_tx,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
            )
            .await;
        }
        CommandAction::RunSpec { path } => {
            let spec = load_spec_file(cwd, &path)?;
            let run = executor.start_imported_spec(spec)?;
            let run = match gate_live_approval(cwd, &store, run, approval_mode, &tui_tx)? {
                LiveApprovalOutcome::Proceed { run, note } => {
                    approval_notes.push(note);
                    run
                }
                LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
                    return Ok(message);
                }
            };
            executor.execute_with_runner(run, &runner).await?
        }
        CommandAction::RunTemplate { name } => {
            let template = load_template(cwd, &name)?;
            let run = match template.harness_source {
                Some(harness) => {
                    if let Ok(v2_plan) = WorkflowV2HarnessValidator.validate(&harness) {
                        let task = template.spec.task.clone();
                        let mut plan = LivePlan {
                            spec: template.spec,
                            harness_source: harness.trim().to_string(),
                            calls: v2_plan.calls,
                        };
                        cap_live_plan_parallelism(&mut plan, &runner);
                        return workflow_live_v2::run_saved_v2_workflow(
                            cwd,
                            &store,
                            plan,
                            task,
                            llm,
                            tui_tx,
                            runner.agent_names.clone(),
                            approval_mode,
                            workspace_boundary_supported,
                        )
                        .await;
                    }
                    executor.start_with_harness(
                        template.spec,
                        &harness,
                        WorkflowBundleOrigin::SavedCommand,
                    )?
                }
                None => executor.start(template.spec)?,
            };
            let run = match gate_live_approval(cwd, &store, run, approval_mode, &tui_tx)? {
                LiveApprovalOutcome::Proceed { run, note } => {
                    approval_notes.push(note);
                    run
                }
                LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
                    return Ok(message);
                }
            };
            executor.execute_with_runner(run, &runner).await?
        }
        CommandAction::Resume { run_id } | CommandAction::Continue { run_id } => {
            if let Some(output) = workflow_live_v2::resume_generated_v2_workflow(
                cwd,
                &store,
                &run_id,
                llm.clone(),
                tui_tx.clone(),
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
            )
            .await?
            {
                return Ok(output);
            }
            let run = store.load_state(&run_id)?;
            if let Some(message) = terminal_resume_message(&run) {
                return Ok(message);
            }
            let run = match gate_live_approval(cwd, &store, run, approval_mode, &tui_tx)? {
                LiveApprovalOutcome::Proceed { run, note } => {
                    approval_notes.push(note);
                    if matches!(run.status, RunStatus::Paused) {
                        LifecycleController::new(store.clone())
                            .apply(&run.id, LifecycleAction::Resume)?
                    } else {
                        run
                    }
                }
                LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
                    return Ok(message);
                }
            };
            executor.execute_with_runner(run, &runner).await?
        }
        other => return run_action(cwd, other),
    };
    let learning_note = workflow_world_learning::record_report(&store, &report);
    let wc_blocks = write_coordination_blocks(&store, &report.run_id);
    let evidence_blocks = workflow_status_blocks::evidence_blocks(&store, &report.run_id);
    let mut output = approval_notes.join("");
    output.push_str(&format!(
        "Workflow complete: {} (completed {}, blocked {}, forced {}, failed {}, skipped {})",
        report.run_id,
        report.completed,
        report.blocked,
        report.forced_accepted,
        report.failed,
        report.skipped
    ));
    output.push('\n');
    output.push_str(&learning_note);
    output.push_str(&wc_blocks);
    output.push_str(&evidence_blocks);
    Ok(output)
}

fn cap_live_plan_parallelism(plan: &mut LivePlan, runner: &PipelineWorkflowRunner) {
    let cap = runner
        .max_concurrency()
        .unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT)
        .max(1) as u32;
    plan.spec.max_parallelism = plan.spec.max_parallelism.min(cap).max(1);
    for stage in &mut plan.spec.stages {
        if let Some(max_parallelism) = stage.max_parallelism {
            stage.max_parallelism = Some(max_parallelism.min(cap).max(1));
        }
    }
}

fn terminal_resume_message(run: &WorkflowRun) -> Option<String> {
    match run.status {
        RunStatus::Failed => {
            let mut message = format!(
                "Workflow {} is failed and cannot be resumed directly.\n",
                run.id
            );
            if let Some(stage_id) = first_stage_with_status(run, StageStatus::Failed) {
                message.push_str(&format!(
                    "Use high-level recovery first:\n/workflow repair {}\n/workflow continue {}\n/workflow restart task {} <task-id>\n\nDebug detail: failed internal stage is {}.\n",
                    run.id, run.id, run.id, stage_id
                ));
            } else {
                message.push_str(&format!(
                    "Use high-level recovery first:\n/workflow repair {}\n/workflow continue {}\n",
                    run.id, run.id
                ));
            }
            Some(message)
        }
        RunStatus::Completed => Some(format!(
            "Workflow {} is already completed; start a new workflow run for new work.\n",
            run.id
        )),
        RunStatus::Cancelled => Some(format!(
            "Workflow {} is cancelled and cannot be resumed; start a new workflow run.\n",
            run.id
        )),
        _ => None,
    }
}

fn first_stage_with_status(run: &WorkflowRun, status: StageStatus) -> Option<&str> {
    run.stages
        .values()
        .find(|stage| stage.status == status)
        .map(|stage| stage.id.as_str())
}

fn live_policy(cwd: &Path, config_path: Option<&Path>) -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::from_config(&load_workflow_config(cwd, config_path))
    }
}

fn load_workflow_config(cwd: &Path, config_path: Option<&Path>) -> WorkflowConfig {
    use archon_core::config_layers::{deep_merge_toml, discover_config_paths};
    let mut merged = toml::Value::Table(toml::map::Map::new());
    for layer in discover_config_paths(config_path, cwd, None) {
        let Ok(text) = std::fs::read_to_string(&layer.path) else {
            continue;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            continue;
        };
        merged = deep_merge_toml(merged, value);
    }
    merged
        .get("workflow")
        .cloned()
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
        .try_into()
        .unwrap_or_else(|_| WorkflowConfig::default())
}

/// TASK-WC-008: render the §17 compact write-coordination status block for
/// every coordinated stage that left state on disk.
fn write_coordination_blocks(store: &WorkflowStore, run_id: &str) -> String {
    use archon_workflow::write_coordinator::status::{
        coordinated_stage_ids, read_status, render_compact,
    };
    let mut out = String::new();
    for stage_id in coordinated_stage_ids(store, run_id) {
        if let Ok(Some(status)) = read_status(store, run_id, &stage_id) {
            out.push_str(&render_compact(&status));
        }
    }
    out
}

fn live_start_message(action: &CommandAction) -> String {
    match action {
        CommandAction::Plan { task } => format!("Planning dynamic workflow for task: {task}\n"),
        CommandAction::PlanSpec { path } => {
            format!("Validating dynamic workflow spec: {path}\n")
        }
        CommandAction::Run { task } => format!("Starting dynamic workflow for task: {task}\n"),
        CommandAction::RunSpec { path } => {
            format!("Starting dynamic workflow from spec: {path}\n")
        }
        CommandAction::RunTemplate { name } => {
            format!("Starting dynamic workflow from template: {name}\n")
        }
        CommandAction::Resume { run_id } => {
            format!("Resuming dynamic workflow {run_id} with the active TUI provider...\n")
        }
        CommandAction::Continue { run_id } => {
            format!("Continuing dynamic workflow {run_id} with the active TUI provider...\n")
        }
        _ => "Starting dynamic workflow...\n".to_string(),
    }
}
