use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, PipelineType, ToolAccessLevel,
};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::{
    CommandAction, ProviderTier, StageKind, StageRunOutput, StageRunRequest, WorkflowExecutor,
    WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

use crate::command::pipeline_support::build_pipeline_adapter;
use crate::command::workflow::{load_spec_file, load_template_spec, run_action};

#[cfg(test)]
#[path = "workflow_live_tests.rs"]
mod tests;
#[path = "workflow_live_prompt.rs"]
mod workflow_live_prompt;

use workflow_live_prompt::{planner_prompt, repair_prompt, workflow_prompt};

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
    Ok(format!(
        "Workflow complete: {} (completed {}, failed {}, skipped {})",
        report.run_id, report.completed, report.failed, report.skipped
    ))
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

struct PipelineWorkflowRunner {
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
}

#[async_trait::async_trait]
impl WorkflowStageRunner for PipelineWorkflowRunner {
    fn max_concurrency(&self) -> Option<usize> {
        // The TUI/live runner routes each fan-out item through the process
        // subagent executor, whose `SubagentManager` has a hard concurrency cap
        // that *rejects* overflow. Query the live executor's authoritative cap
        // (derived from config.subagent.max_concurrent) so fan-out width clamps
        // to it and extra items wait for a slot instead of failing with "max
        // concurrent subagents reached". Falls back to the default constant when
        // no executor is installed (e.g. CLI paths before session bootstrap).
        let from_executor = archon_tools::subagent_executor::get_subagent_executor()
            .and_then(|exec| exec.max_concurrency());
        Some(
            from_executor.unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT),
        )
    }

    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let model_alias = tier_model_alias(request.provider_tier).to_string();
        let resolved_model = self.llm.resolve_model_alias(&model_alias);
        let provider_id = self
            .llm
            .provider_id()
            .unwrap_or_else(|| "active-provider".to_string());
        let agent = workflow_agent(&request, &model_alias);
        self.emit_activity(
            &request,
            &provider_id,
            &resolved_model,
            AgentActivityStatus::Running,
            "stage running",
        );
        let response = self
            .llm
            .run_agent(AgentExecutionRequest {
                session_id: request.run_id.clone(),
                pipeline_type: PipelineType::Workflow,
                task: request.task.clone(),
                cwd: request_target_repository_root(&request),
                ordinal: request.attempt as usize,
                attempt: request.attempt as usize,
                agent,
                messages: vec![serde_json::json!({
                    "role": "user",
                    "content": workflow_prompt(&request),
                })],
                system: vec![serde_json::json!({
                    "type": "text",
                    "text": "You are an Archon dynamic workflow stage agent. Return only useful public output for the stage artifact. Do not include private reasoning, hidden chain-of-thought, credentials, or provider internals.",
                })],
                tools: Vec::new(),
                allowed_tools: allowed_tools(&request),
            })
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                self.emit_activity(
                    &request,
                    &provider_id,
                    &resolved_model,
                    AgentActivityStatus::Failed,
                    "stage failed",
                );
                return Err(archon_workflow::WorkflowError::StageFailed(err.to_string()));
            }
        };
        self.emit_activity(
            &request,
            &provider_id,
            &resolved_model,
            AgentActivityStatus::Complete,
            "stage complete",
        );
        let mut output = StageRunOutput::markdown(response.content);
        output.provider_id = Some(provider_id);
        output.resolved_model = Some(resolved_model);
        output.tokens_in = response.tokens_in;
        output.tokens_out = response.tokens_out;
        Ok(output)
    }
}

fn request_target_repository_root(request: &StageRunRequest) -> Option<PathBuf> {
    request
        .input
        .get("target_repository_root")
        .and_then(|value| value.as_str())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

impl PipelineWorkflowRunner {
    fn emit_activity(
        &self,
        request: &StageRunRequest,
        provider_id: &str,
        model: &str,
        status: AgentActivityStatus,
        detail: &str,
    ) {
        let name = request
            .agent
            .clone()
            .unwrap_or_else(|| request.stage_id.clone());
        let _ = self
            .tui_tx
            .send(TuiEvent::AgentActivity(AgentActivityUpdate {
                id: format!("workflow:{}:{}", request.run_id, request.stage_id),
                name,
                role: AgentActivityRole::Subagent,
                status,
                current_tool: None,
                detail: Some(format!(
                    "{detail} provider_tier={:?}",
                    request.provider_tier
                )),
                run_id: Some(request.run_id.clone()),
                parent_id: None,
                artifact_id: None,
                provider: Some(provider_id.to_string()),
                model: Some(model.to_string()),
                cost_usd: None,
            }));
    }
}

fn workflow_agent(request: &StageRunRequest, model: &str) -> AgentInfo {
    let key = request
        .agent
        .clone()
        .unwrap_or_else(|| format!("workflow-{}", request.stage_id));
    AgentInfo {
        display_name: key.replace('-', " "),
        key,
        model: model.to_string(),
        phase: 0,
        critical: matches!(request.stage_kind, StageKind::QualityGate),
        parallelizable: matches!(request.stage_kind, StageKind::Fanout),
        quality_threshold: 0.5,
        tool_access_level: if matches!(request.stage_kind, StageKind::Implementation)
            || command_execution_stage(request)
        {
            ToolAccessLevel::Full
        } else {
            ToolAccessLevel::ReadOnly
        },
    }
}

fn allowed_tools(request: &StageRunRequest) -> Vec<String> {
    let tools = match request.stage_kind {
        StageKind::Implementation => vec![
            "Read",
            "Grep",
            "Glob",
            "Write",
            "Edit",
            "ApplyPatch",
            "LargeEditBegin",
            "LargeEditInsertAfter",
            "LargeEditReplaceSection",
            "LargeEditDeleteSection",
            "LargeEditCommit",
            "LargeEditAbort",
            "Bash",
        ],
        _ if command_execution_stage(request) => {
            vec!["Read", "Grep", "Glob", "Bash", "DocSearch", "DocGet"]
        }
        StageKind::Tool => vec!["Read", "Grep", "Glob", "DocSearch", "DocGet"],
        _ => vec![
            "Read",
            "Grep",
            "Glob",
            "WebSearch",
            "WebFetch",
            "DocSearch",
            "DocGet",
        ],
    };
    tools.into_iter().map(str::to_string).collect()
}

fn command_execution_stage(request: &StageRunRequest) -> bool {
    if stage_extra_requests_bash(request) {
        return true;
    }
    let haystack = format!(
        "{}\n{}\n{}",
        request.stage_id,
        request.task,
        request
            .input
            .get("stage_task")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    [
        "focused_test",
        "focused-test",
        "focused test",
        "cargo test",
        "test command",
        "run tests",
        "verification",
        "verify",
        "quality gate",
        "cargo check",
        "cargo build",
        "cargo fmt",
        "rustfmt",
        "clippy",
        "lint",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn stage_extra_requests_bash(request: &StageRunRequest) -> bool {
    let Some(extra) = request.input.get("stage_extra") else {
        return false;
    };
    ["allowed_tools", "tools", "required_tools"]
        .iter()
        .filter_map(|key| extra.get(*key))
        .flat_map(text_values)
        .any(|tool| tool.eq_ignore_ascii_case("bash") || tool.eq_ignore_ascii_case("shell"))
}

fn text_values(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(value) => vec![value.as_str()],
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_yaml(content: &str) -> String {
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        let after = after.strip_prefix("yaml").unwrap_or(after);
        let after = after.strip_prefix('\n').unwrap_or(after);
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    content.trim().to_string()
}

fn tier_model_alias(tier: ProviderTier) -> &'static str {
    match tier {
        ProviderTier::Cheap | ProviderTier::Local => "haiku",
        ProviderTier::Critic | ProviderTier::Reducer => "opus",
        ProviderTier::Planner
        | ProviderTier::Researcher
        | ProviderTier::Coder
        | ProviderTier::Vision => "sonnet",
    }
}
