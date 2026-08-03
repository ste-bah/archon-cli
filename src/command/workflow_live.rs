use anyhow::{Result, anyhow};
use archon_core::agents::AgentRegistry;
use archon_core::config::{ArchonConfig, GeneratedWorkflowConfig};
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{
    CommandAction, RunStatus, SharedWorkflowUiSink, StageStatus, WorkflowConfig, WorkflowLlmClient,
    WorkflowLlmClientFactory, WorkflowLlmClientRequest, WorkflowPolicy, WorkflowRun,
    WorkflowStageRunner, WorkflowStore, WorkflowUiEvent,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink;
use crate::command::workflow::{load_spec_file, load_template, run_action};
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
#[path = "workflow_live_generated_lifecycle_remediation.rs"]
mod workflow_live_generated_lifecycle_remediation;
#[path = "workflow_live_generated_lifecycle_support.rs"]
mod workflow_live_generated_lifecycle_support;
#[path = "workflow_live_generated_scaffold.rs"]
pub(crate) mod workflow_live_generated_scaffold;
#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests.rs"]
mod workflow_live_generated_semantics_tests;
#[path = "workflow_live_items.rs"]
mod workflow_live_items;
#[path = "workflow_live_mcp.rs"]
mod workflow_live_mcp;
#[path = "workflow_live_planner.rs"]
mod workflow_live_planner;
#[path = "workflow_live_prompt.rs"]
mod workflow_live_prompt;
#[path = "workflow_live_repo_root.rs"]
mod workflow_live_repo_root;
#[path = "workflow_live_retry.rs"]
mod workflow_live_retry;
#[path = "workflow_live_runner.rs"]
mod workflow_live_runner;
#[path = "workflow_live_runner_activity.rs"]
mod workflow_live_runner_activity;
#[cfg(test)]
#[path = "workflow_live_runner_tests.rs"]
mod workflow_live_runner_tests;
#[cfg(test)]
#[path = "workflow_live_runtime_genericity_tests.rs"]
mod workflow_live_runtime_genericity_tests;
#[path = "workflow_live_semantic_preservation.rs"]
mod workflow_live_semantic_preservation;
#[path = "workflow_live_shape_apply.rs"]
mod workflow_live_shape_apply;
// The task universe itself moved to `archon_workflow::task_universe`. What
// stayed is this half of its tests: they assert against
// `workflow_live_generated_lifecycle_support`, which is still in this crate,
// so they cannot live beside the code they cover until it moves too.
#[cfg(test)]
#[path = "workflow_live_task_status_tests.rs"]
mod workflow_live_task_status_tests;
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
// Orchestrated lifecycle (v3): persistent-orchestrator action layer. The
// driver wiring lands incrementally; the action contract compiles and is
// tested from day one.
#[path = "workflow_live_v3_orchestrator_actions.rs"]
mod workflow_live_v3_orchestrator_actions;
#[path = "workflow_live_verification_contract.rs"]
mod workflow_live_verification_contract;
#[cfg(test)]
#[path = "workflow_v2_live_tests.rs"]
mod workflow_v2_live_tests;

use workflow_live_config_layers::{
    live_policy, load_generated_workflow_config, load_learning_config,
};
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
) {
    archon_observability::spawn_named("dynamic-workflow-run", async move {
        if let Err(error) = ui_sink
            .emit(WorkflowUiEvent::Text(live_start_message(&action)))
            .await
        {
            tracing::error!(%error, "workflow start notification delivery failed");
            return;
        }
        let generated_config = load_generated_workflow_config(&cwd, config_path.as_deref());
        let result = run_live_action(
            &cwd,
            action,
            llm,
            ui_sink.clone(),
            config_path,
            generated_config,
            true,
            LiveApprovalMode::InteractiveSurface,
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
}

pub(crate) async fn run_live_cli_action(
    cwd: &Path,
    action: CommandAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    llm_factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    let llm = llm_factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow_cli".to_string(),
            session_id: "workflow-cli".to_string(),
        })
        .await?;
    // The one place in this file that still names the TUI. A CLI run has no
    // terminal UI attached, but it must still exert the same backpressure and
    // coalescing a TUI run does, or the two paths would differ in exactly the
    // conditions that produce bugs. So the CLI builds the real channel and
    // drains it, and passes the sender through the same port a TUI run uses.
    let (tui_tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(128);
    let ui_sink = TuiWorkflowUiSink::arc(tui_tx);
    let drain = archon_observability::spawn_named("workflow-cli-tui-drain", async move {
        while rx.recv().await.is_some() {}
    });
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(archon_core::config::default_config_path);
    let result = run_live_action(
        cwd,
        action,
        llm,
        ui_sink,
        Some(config_path),
        config.workflow.generated.clone(),
        true,
        LiveApprovalMode::CliYes,
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

async fn run_live_action(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    config_path: Option<PathBuf>,
    generated_config: GeneratedWorkflowConfig,
    workspace_boundary_supported: bool,
    approval_mode: LiveApprovalMode,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let policy = live_policy(cwd, config_path.as_deref());
    let learning = load_learning_config(cwd, config_path.as_deref());
    // The one place a generated run's limits are decided. SONA is consulted
    // here rather than inside the planner because every downstream consumer —
    // the lifecycle driver's repair caps, the host-call client's timeout, the
    // read-only branch budget, and the metadata a resume replays — reads the
    // same `generated_config`, so a single substitution reaches all of them and
    // no path can end up with a half-tuned config.
    let task_class = live_task_class(&action);
    let mut generated_config = generated_config;
    let mut tuning_decisions = Vec::new();
    if let Some(class) = task_class {
        let tuning = crate::command::sona_workflow_tuning::tune_generated_config(
            cwd,
            class,
            &learning,
            &generated_config,
        );
        let report = tuning.report(class);
        generated_config = tuning.config;
        tuning_decisions = tuning.decisions;
        if !report.is_empty() {
            tracing::info!(class, %report, "generated limits tuned by SONA");
            // Emitted before any work starts: a user who wonders why this run
            // got five repair iterations must be able to read the answer in the
            // run's own output rather than reconstruct it from the learning
            // store by hand.
            if let Err(error) = ui_sink.emit(WorkflowUiEvent::Text(report)).await {
                tracing::debug!(%error, "tuning report delivery failed");
            }
        }
    }
    let generated_config = generated_config;
    let tuning_decisions = tuning_decisions;
    let runner = PipelineWorkflowRunner {
        llm: llm.clone(),
        ui_sink: ui_sink.clone(),
        agent_names: AgentRegistry::load(cwd)
            .available_agent_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
        workspace_boundary_supported,
    };
    let capped_live_plan = |task: &str| {
        let llm = llm.clone();
        let ui_sink = ui_sink.clone();
        let task = task.to_string();
        let store = &store;
        let runner = &runner;
        let policy = &policy;
        let generated_config = &generated_config;
        let learning = &learning;
        let tuning_decisions = &tuning_decisions;
        async move {
            let mut plan = plan_live(
                store,
                &task,
                llm,
                ui_sink.clone(),
                generated_config,
                learning,
            )
            .await?;
            cap_live_plan_parallelism(&mut plan, runner, policy);
            // Attached here rather than inside the planner because the planner
            // never sees the baseline it was tuned away from, and a decision
            // record without its baseline explains nothing.
            plan.tuning_decisions = tuning_decisions.clone();
            // Shape comes after the plan, not before it like the budgets: the
            // knob is scored against the plan's own stage families and the
            // declared task graph, and neither exists until the planner has
            // run. Budgets have no such dependency, which is why they are
            // resolved earlier and reach the planner itself.
            apply_generated_shape(cwd, task_class, learning, &mut plan, &ui_sink).await;
            Ok::<_, anyhow::Error>(plan)
        }
    };
    match action {
        CommandAction::Plan { task } => {
            let plan = capped_live_plan(&task).await?;
            render_live_plan(&plan)
        }
        CommandAction::PlanSpec { path } => Ok(load_spec_file(cwd, &path)?.to_yaml()?),
        CommandAction::Run { task, decomposed } => {
            let plan = capped_live_plan(&task).await?;
            return workflow_live_v2::run_generated_v2_workflow(
                cwd,
                &store,
                plan,
                task,
                llm,
                ui_sink,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                if decomposed {
                    false
                } else {
                    workflow_live_v2::script_lifecycle_from_env()
                },
                &learning,
            )
            .await;
        }
        CommandAction::RunSpec { .. } => Err(anyhow!(
            "legacy imported-spec execution was removed by the workflow runtime rescue;                  run work through the V2 runtime with /workflow run <task> or a saved V2 workflow"
        )),
        CommandAction::RunTemplate { name, args } => {
            let template = load_template(cwd, &name)?;
            let Some(harness) = template.harness_source else {
                return Err(anyhow!(
                    "saved workflow '{name}' has no V2 harness; legacy template execution                      was removed by the workflow runtime rescue"
                ));
            };
            // QuickJS dry-run is the single grammar; failure is a hard error.
            let calls = workflow_live_v2::dry_run_workflow_plan(&harness, args.as_ref())
                .await
                .map_err(|err| anyhow!("saved workflow '{name}' failed validation: {err}"))?;
            let task = template.spec.task.clone();
            let mut plan = WorkflowScriptPlan::from_template(template.spec, &harness, calls);
            plan.script_args = args;
            cap_live_plan_parallelism(&mut plan, &runner, &policy);
            return workflow_live_v2::run_saved_v2_workflow(
                cwd,
                &store,
                plan,
                task,
                llm,
                ui_sink,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                &learning,
            )
            .await;
        }
        CommandAction::Resume { run_id } | CommandAction::Continue { run_id } => {
            if let Some(output) = workflow_live_v2::resume_generated_v2_workflow(
                cwd,
                &store,
                &run_id,
                llm.clone(),
                ui_sink.clone(),
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                &learning,
            )
            .await?
            {
                return Ok(output);
            }
            let run = store.load_state(&run_id)?;
            if let Some(message) = terminal_resume_message(&run) {
                return Ok(message);
            }
            Err(anyhow!(
                "workflow {run_id} is not a resumable V2 run; legacy stage execution was                  removed by the workflow runtime rescue"
            ))
        }
        other => run_action(cwd, other),
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
