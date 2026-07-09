use anyhow::{Result, anyhow};
use archon_core::agents::AgentRegistry;
use archon_core::config::{ArchonConfig, GeneratedWorkflowConfig};
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    CommandAction, RunStatus, StageStatus, WorkflowConfig, WorkflowPolicy, WorkflowRun,
    WorkflowStageRunner, WorkflowStore,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::command::pipeline_support::build_subagent_pipeline_adapter;
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
#[cfg(test)]
#[path = "workflow_live_execution_tests.rs"]
mod workflow_live_execution_tests;
#[path = "workflow_live_generated_contract.rs"]
mod workflow_live_generated_contract;
#[path = "workflow_live_generated_lifecycle_remediation.rs"]
mod workflow_live_generated_lifecycle_remediation;
#[path = "workflow_live_generated_lifecycle_support.rs"]
mod workflow_live_generated_lifecycle_support;
#[path = "workflow_live_generated_scaffold.rs"]
mod workflow_live_generated_scaffold;
#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests.rs"]
mod workflow_live_generated_semantics_tests;
#[path = "workflow_live_items.rs"]
mod workflow_live_items;
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
#[cfg(test)]
#[path = "workflow_live_runner_tests.rs"]
mod workflow_live_runner_tests;
#[path = "workflow_live_task_universe.rs"]
mod workflow_live_task_universe;
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
#[path = "workflow_live_verification_contract.rs"]
mod workflow_live_verification_contract;
#[cfg(test)]
#[path = "workflow_v2_live_tests.rs"]
mod workflow_v2_live_tests;

use workflow_live_planner::{WorkflowScriptPlan, plan_live, render_live_plan};
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
        let generated_config = load_generated_workflow_config(&cwd, config_path.as_deref());
        let result = run_live_action(
            &cwd,
            action,
            llm,
            tui_tx.clone(),
            config_path,
            generated_config,
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
    let llm =
        build_subagent_pipeline_adapter(config, env_vars, "workflow_cli", cwd, "workflow-cli")
            .await?;
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
        config.workflow.generated.clone(),
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
    generated_config: GeneratedWorkflowConfig,
    workspace_boundary_supported: bool,
    approval_mode: LiveApprovalMode,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let policy = live_policy(cwd, config_path.as_deref());
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
    let capped_live_plan = |task: &str| {
        let llm = llm.clone();
        let tui_tx = tui_tx.clone();
        let task = task.to_string();
        let store = &store;
        let runner = &runner;
        let policy = &policy;
        let generated_config = &generated_config;
        async move {
            let mut plan = plan_live(store, &task, llm, tui_tx, generated_config).await?;
            cap_live_plan_parallelism(&mut plan, runner, policy);
            Ok::<_, anyhow::Error>(plan)
        }
    };
    match action {
        CommandAction::Plan { task } => {
            let plan = capped_live_plan(&task).await?;
            render_live_plan(&plan)
        }
        CommandAction::PlanSpec { path } => Ok(load_spec_file(cwd, &path)?.to_yaml()?),
        CommandAction::Run { task } => {
            let plan = capped_live_plan(&task).await?;
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
                tui_tx,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
            )
            .await;
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

fn load_generated_workflow_config(
    cwd: &Path,
    config_path: Option<&Path>,
) -> GeneratedWorkflowConfig {
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
        .and_then(|workflow| workflow.get("generated"))
        .cloned()
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
        .try_into()
        .unwrap_or_else(|_| GeneratedWorkflowConfig::default())
}

/// Render compact write-coordination status blocks left on disk.
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
