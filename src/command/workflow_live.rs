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
    CommandAction, ProviderTier, WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStore,
};

use crate::command::pipeline_support::build_pipeline_adapter;
use crate::command::workflow::{load_spec_file, load_template_spec, run_action};
use crate::command::workflow_world_learning;

#[cfg(test)]
#[path = "workflow_live_tests.rs"]
mod tests;
#[path = "workflow_agent_select.rs"]
mod workflow_agent_select;
#[path = "workflow_live_prompt.rs"]
mod workflow_live_prompt;
#[path = "workflow_live_retry.rs"]
mod workflow_live_retry;
#[path = "workflow_live_runner.rs"]
mod workflow_live_runner;

use workflow_live_prompt::{planner_prompt, repair_prompt};
use workflow_live_runner::{PipelineWorkflowRunner, extract_yaml, tier_model_alias};

pub(crate) fn should_spawn_live(action: &CommandAction) -> bool {
    matches!(
        action,
        CommandAction::Plan { .. }
            | CommandAction::Run { .. }
            | CommandAction::RunSpec { .. }
            | CommandAction::RunTemplate { .. }
            | CommandAction::Resume { .. }
    )
}

pub(crate) fn spawn_live_workflow(
    cwd: PathBuf,
    action: CommandAction,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
) {
    let _ = tui_tx.send(TuiEvent::TextDelta(live_start_message(&action)));
    archon_observability::spawn_named("dynamic-workflow-run", async move {
        let result = run_live_action(&cwd, action, llm, tui_tx.clone()).await;
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
    run_live_action(cwd, action, llm, tui_tx).await
}

async fn run_live_action(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let runner = PipelineWorkflowRunner {
        llm: llm.clone(),
        tui_tx: tui_tx.clone(),
        agent_names: AgentRegistry::load(cwd)
            .available_agent_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
    };
    let report = match action {
        CommandAction::Plan { task } => {
            let spec = plan_live(&task, llm, tui_tx).await?;
            return Ok(spec.to_yaml()?);
        }
        CommandAction::PlanSpec { path } => return Ok(load_spec_file(cwd, &path)?.to_yaml()?),
        CommandAction::Run { task } => {
            let spec = plan_live(&task, llm, tui_tx).await?;
            let run = executor.start(spec)?;
            executor.execute_with_runner(run, &runner).await?
        }
        CommandAction::RunSpec { path } => {
            let spec = load_spec_file(cwd, &path)?;
            let run = executor.start(spec)?;
            executor.execute_with_runner(run, &runner).await?
        }
        CommandAction::RunTemplate { name } => {
            let spec = load_template_spec(cwd, &name)?;
            let run = executor.start(spec)?;
            executor.execute_with_runner(run, &runner).await?
        }
        CommandAction::Resume { run_id } => {
            let run = store.load_state(&run_id)?;
            executor.execute_with_runner(run, &runner).await?
        }
        other => return run_action(cwd, other),
    };
    let learning_note = workflow_world_learning::record_report(&store, &report);
    let wc_blocks = write_coordination_blocks(&store, &report.run_id);
    Ok(format!(
        "Workflow complete: {} (completed {}, failed {}, skipped {})",
        report.run_id, report.completed, report.failed, report.skipped
    ) + "\n"
        + learning_note.as_str()
        + wc_blocks.as_str())
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

async fn plan_live(
    task: &str,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
) -> Result<WorkflowSpec> {
    match llm_plan(task, llm).await {
        Ok(spec) => Ok(spec),
        Err(err) => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Workflow planner failed validation; live mode will not fall back to a deterministic smoke plan: {err}\n"
            )));
            Err(err)
        }
    }
}

async fn llm_plan(task: &str, llm: Arc<dyn LlmClient>) -> Result<WorkflowSpec> {
    let response = llm
        .send_message(
            vec![serde_json::json!({
                "role": "user",
                "content": planner_prompt(task),
            })],
            vec![serde_json::json!({
                "type": "text",
                "text": "You are Archon's provider-neutral dynamic workflow planner. Return only valid YAML for the requested schema. Do not include hidden reasoning, credentials, provider names, or model names.",
            })],
            Vec::new(),
            tier_model_alias(ProviderTier::Planner),
        )
        .await?;
    let raw = extract_yaml(&response.content);
    match WorkflowSpec::from_generated_yaml(&raw, task) {
        Ok(spec) => Ok(spec),
        Err(err) => repair_plan(task, &raw, err.to_string(), llm).await,
    }
}

async fn repair_plan(
    task: &str,
    invalid_yaml: &str,
    error: String,
    llm: Arc<dyn LlmClient>,
) -> Result<WorkflowSpec> {
    let response = llm
        .send_message(
            vec![serde_json::json!({
                "role": "user",
                "content": repair_prompt(task, invalid_yaml, &error),
            })],
            vec![serde_json::json!({
                "type": "text",
                "text": "Repair the workflow YAML only. Preserve provider neutrality and remove invalid fields.",
            })],
            Vec::new(),
            tier_model_alias(ProviderTier::Planner),
        )
        .await?;
    WorkflowSpec::from_generated_yaml(&extract_yaml(&response.content), task).map_err(Into::into)
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
        _ => "Starting dynamic workflow...\n".to_string(),
    }
}
