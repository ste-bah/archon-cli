use anyhow::{Result, anyhow};
use archon_core::agents::AgentRegistry;
use archon_core::config::{ArchonConfig, GeneratedWorkflowConfig};
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{
    CommandAction, RunStatus, SharedWorkflowUiSink, StageStatus, WorkflowLlmClient,
    WorkflowLlmClientFactory, WorkflowLlmClientRequest, WorkflowPolicy, WorkflowRun,
    WorkflowStageRunner, WorkflowStore, WorkflowUiEvent,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink;
#[path = "workflow_live_action.rs"]
mod action;
use crate::command::workflow::{load_spec_file, load_template, run_action};
#[cfg(test)]
use action::run_live_action;
use action::run_live_action_with_policy;
#[cfg(test)]
#[path = "workflow_live_planner_repair_tests.rs"]
mod planner_repair_tests;
#[cfg(test)]
#[path = "workflow_live_tests.rs"]
mod tests;
#[path = "workflow_live_approval.rs"]
mod workflow_live_approval;
#[path = "workflow_live_board.rs"]
mod workflow_live_board;
#[cfg(test)]
#[path = "workflow_live_canary_retry_tests.rs"]
mod workflow_live_canary_retry_tests;
#[cfg(test)]
#[path = "workflow_live_canary_tests.rs"]
mod workflow_live_canary_tests;
#[path = "workflow_live_config_layers.rs"]
mod workflow_live_config_layers;
#[cfg(test)]
#[path = "workflow_live_execution_tests.rs"]
mod workflow_live_execution_tests;
#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests.rs"]
mod workflow_live_generated_semantics_tests;
#[path = "workflow_live_mcp.rs"]
mod workflow_live_mcp;
#[path = "workflow_live_planner.rs"]
pub(crate) mod workflow_live_planner;
#[path = "workflow_live_repository.rs"]
pub(crate) mod workflow_live_repository;
#[path = "workflow_live_retry.rs"]
mod workflow_live_retry;
#[path = "workflow_live_runner.rs"]
pub(crate) mod workflow_live_runner;
#[cfg(test)]
#[path = "workflow_live_runner_tests.rs"]
mod workflow_live_runner_tests;
#[cfg(test)]
#[path = "workflow_live_runtime_genericity_tests.rs"]
mod workflow_live_runtime_genericity_tests;
#[path = "workflow_live_shape_apply.rs"]
mod workflow_live_shape_apply;
#[cfg(test)]
#[path = "workflow_live_test_support.rs"]
mod workflow_live_test_support;
#[path = "workflow_live_v2.rs"]
mod workflow_live_v2;
#[cfg(test)]
pub(crate) use workflow_live_v2::workflow_run_end_snapshot::recover_bound_task_set;
pub(crate) use workflow_live_v2::{
    execute_fixed_decomposition_v2_run, reobserve, save_fixed_decomposition_metadata,
};
// #189 Phase 4: real tool calls from inside a workflow script.
#[path = "workflow_script_tools.rs"]
pub(crate) mod workflow_script_tools;
#[cfg(test)]
#[path = "workflow_v2_live_tests.rs"]
mod workflow_v2_live_tests;

use workflow_live_config_layers::{live_policy, load_learning_config};
use workflow_live_planner::{WorkflowScriptPlan, plan_live, render_live_plan};
use workflow_live_runner::PipelineWorkflowRunner;
use workflow_live_shape_apply::{apply_generated_shape, live_task_class};

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
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    config_path: Option<PathBuf>,
    config: ArchonConfig,
) -> Result<()> {
    let check_policy =
        crate::command::acceptance_check_policy::for_config_action(&action, &config)?;
    archon_observability::spawn_named("dynamic-workflow-run", async move {
        if let Err(error) = ui_sink
            .emit(WorkflowUiEvent::Text(live_start_message(&action)))
            .await
        {
            tracing::error!(%error, "workflow start notification delivery failed");
            return;
        }
        let generated_config = config.workflow.generated.clone();
        let result = run_live_action_with_policy(
            &cwd,
            action,
            llm,
            ui_sink.clone(),
            config_path,
            generated_config,
            true,
            LiveApprovalMode::InteractiveSurface,
            check_policy,
        )
        .await;
        match result {
            Ok(text) => {
                if let Err(error) = ui_sink.emit(WorkflowUiEvent::Text(text)).await {
                    tracing::error!(%error, "workflow completion notification delivery failed");
                }
            }
            Err(err) => {
                let message = format!("Workflow failed: {err}");
                if let Err(error) = ui_sink
                    .emit(WorkflowUiEvent::Text(format!("{message}\n")))
                    .await
                {
                    tracing::error!(%error, "workflow failure text delivery failed");
                    return;
                }
                if let Err(error) = ui_sink.emit(WorkflowUiEvent::Error(message)).await {
                    tracing::error!(%error, "workflow failure notification delivery failed");
                }
            }
        }
    });
    Ok(())
}

pub(crate) async fn run_live_cli_action(
    cwd: &Path,
    action: CommandAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    llm_factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    let check_policy = crate::command::acceptance_check_policy::for_config_action(&action, config)?;
    // Before anything that can run a stage. A CLI workflow builds no session, so
    // this is the only place in the process that installs the process-global
    // board — without it the stage subagents' board tools report the board as
    // offline and the lifecycle's drain gate has nothing to read (#142). It is
    // deliberately not in `run_live_action`: the TUI reaches that function too,
    // and there the board was already installed by `interactive_bootstrap` from
    // a `MemoryAccess` this process holds.
    workflow_live_board::install_workflow_board_access(config).await;
    let llm = llm_factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow_cli".to_string(),
            session_id: "workflow-cli".to_string(),
            read_roots: Vec::new(),
        })
        .await?;
    // The one place in this file that still names the TUI. A CLI run has no
    // terminal UI attached, but it must still exert the same backpressure and
    // coalescing a TUI run does, or the two paths would differ in exactly the
    // conditions that produce bugs. So the CLI builds the real channel and
    // drains it, and passes the sender through the same port a TUI run uses.
    let (tui_tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(128);
    // Resilient: a drain task that dies (or a detached TUI) must degrade the
    // run to quiet, never fail the branch that happened to be emitting. Three
    // overnight run halts were exactly that failure mode.
    let ui_sink = archon_workflow::ui_sink_port::ResilientWorkflowUiSink::wrap(
        TuiWorkflowUiSink::arc(tui_tx),
    );
    let drain = archon_observability::spawn_named("workflow-cli-tui-drain", async move {
        while rx.recv().await.is_some() {}
    });
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(archon_core::config::default_config_path);
    let result = run_live_action_with_policy(
        cwd,
        action,
        llm,
        ui_sink,
        Some(config_path),
        config.workflow.generated.clone(),
        true,
        LiveApprovalMode::CliYes,
        check_policy,
    )
    .await;
    drain.abort();
    result
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

fn cap_live_plan_parallelism(
    plan: &mut WorkflowScriptPlan,
    runner: &PipelineWorkflowRunner,
    policy: &WorkflowPolicy,
) {
    let cap = runner
        .max_concurrency()
        .unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT)
        .max(1) as u32;
    plan.max_parallelism = match plan.max_parallelism {
        0 => cap,
        requested => requested.min(cap).max(1),
    };
    let max_agents = policy.max_agents_per_run.max(1);
    plan.max_agents = match plan.max_agents {
        0 => max_agents,
        requested => requested.min(max_agents).max(1),
    };
    for call in &mut plan.calls {
        if let Some(max_parallelism) = call.options.max_parallelism {
            call.options.max_parallelism = Some(max_parallelism.min(cap as usize).max(1));
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
        RunStatus::Cancelled => None,
        _ => None,
    }
}

fn first_stage_with_status(run: &WorkflowRun, status: StageStatus) -> Option<&str> {
    run.stages
        .values()
        .find(|stage| stage.status == status)
        .map(|stage| stage.id.as_str())
}

/// Render compact write-coordination status blocks left on disk.
fn live_start_message(action: &CommandAction) -> String {
    match action {
        CommandAction::Plan { task } => format!("Planning dynamic workflow for task: {task}\n"),
        CommandAction::PlanSpec { path } => {
            format!("Validating dynamic workflow spec: {path}\n")
        }
        CommandAction::Run { task, .. } => format!("Starting dynamic workflow for task: {task}\n"),
        CommandAction::RunSpec { path } => {
            format!("Starting dynamic workflow from spec: {path}\n")
        }
        CommandAction::RunTemplate { name, args } => {
            if args.is_some() {
                format!("Starting dynamic workflow from template: {name} with args\n")
            } else {
                format!("Starting dynamic workflow from template: {name}\n")
            }
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

#[cfg(test)]
#[path = "workflow_audit_test_support.rs"]
pub(crate) mod audit_test_support;
#[cfg(test)]
#[path = "workflow_criterion_results_test_support.rs"]
pub(crate) mod criterion_results_test_support;
