use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, PipelineType, ToolAccessLevel,
};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::{
    CommandAction, HeuristicWorkflowPlanner, ProviderTier, StageKind, StageRunOutput,
    StageRunRequest, WorkflowExecutor, WorkflowPlanner, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

use crate::command::workflow::run_action;

pub(crate) fn should_spawn_live(action: &CommandAction) -> bool {
    matches!(
        action,
        CommandAction::Plan { .. } | CommandAction::Run { .. } | CommandAction::Resume { .. }
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
                let _ = tui_tx.send(TuiEvent::Error(format!("Workflow failed: {err}")));
            }
        }
    });
}

async fn run_live_action(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let runner = PipelineWorkflowRunner {
        llm: llm.clone(),
        tui_tx: tui_tx.clone(),
    };
    let report = match action {
        CommandAction::Plan { task } => {
            let spec = plan_live(&task, llm, tui_tx).await?;
            return Ok(spec.to_yaml()?);
        }
        CommandAction::Run { task } => {
            let spec = plan_live(&task, llm, tui_tx).await?;
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
                "Workflow planner fell back to heuristic plan after validation failure: {err}\n"
            )));
            HeuristicWorkflowPlanner.plan(task).map_err(Into::into)
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
    match WorkflowSpec::from_yaml(&raw) {
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
    WorkflowSpec::from_yaml(&extract_yaml(&response.content)).map_err(Into::into)
}

fn live_start_message(action: &CommandAction) -> String {
    match action {
        CommandAction::Plan { task } => format!("Planning dynamic workflow for task: {task}\n"),
        CommandAction::Run { task } => format!("Starting dynamic workflow for task: {task}\n"),
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
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let model = tier_model_alias(request.provider_tier).to_string();
        let agent = workflow_agent(&request, &model);
        self.emit_activity(
            &request,
            &model,
            AgentActivityStatus::Running,
            "stage running",
        );
        let response = self
            .llm
            .run_agent(AgentExecutionRequest {
                session_id: request.run_id.clone(),
                pipeline_type: PipelineType::Workflow,
                task: request.task.clone(),
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
                    &model,
                    AgentActivityStatus::Failed,
                    "stage failed",
                );
                return Err(archon_workflow::WorkflowError::StageFailed(err.to_string()));
            }
        };
        self.emit_activity(
            &request,
            &model,
            AgentActivityStatus::Complete,
            "stage complete",
        );
        let mut output = StageRunOutput::markdown(response.content);
        output.resolved_model = Some(model);
        output.tokens_in = response.tokens_in;
        output.tokens_out = response.tokens_out;
        Ok(output)
    }
}

impl PipelineWorkflowRunner {
    fn emit_activity(
        &self,
        request: &StageRunRequest,
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
                provider: Some("active-provider".to_string()),
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
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

fn workflow_prompt(request: &StageRunRequest) -> String {
    format!(
        "## Workflow Task\n{}\n\n## Stage\nid: {}\nkind: {:?}\nprovider_tier: {:?}\nattempt: {}\ndepends_on: {:?}\n\n## Stage Input\n{}",
        request.task,
        request.stage_id,
        request.stage_kind,
        request.provider_tier,
        request.attempt,
        request.depends_on,
        request.input
    )
}

fn planner_prompt(task: &str) -> String {
    format!(
        "Create an archon.workflow.v1 YAML plan for this task:\n\n{task}\n\nRules:\n- Use schema: archon.workflow.v1.\n- Use stage kinds: agent, fanout, reduce, tool, checkpoint, quality_gate, human_gate.\n- Use provider_tier aliases only: planner, researcher, coder, critic, cheap, vision, local, reducer.\n- Do not set stage.provider or stage.model.\n- Include at least discovery, fanout/review, reduce/synthesis, and quality gate stages.\n- Keep max_parallelism <= 8 and max_agents <= 200.\n- Add learning_hooks for sona, reasoning_bank, and world_model.\n- Return YAML only."
    )
}

fn repair_prompt(task: &str, invalid_yaml: &str, error: &str) -> String {
    format!(
        "The workflow YAML failed validation.\n\nTask:\n{task}\n\nError:\n{error}\n\nInvalid YAML:\n```yaml\n{invalid_yaml}\n```\n\nReturn repaired archon.workflow.v1 YAML only."
    )
}

fn allowed_tools(request: &StageRunRequest) -> Vec<String> {
    match request.stage_kind {
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
    }
    .into_iter()
    .map(str::to_string)
    .collect()
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

#[cfg(test)]
mod tests {
    use super::extract_yaml;

    #[test]
    fn extract_yaml_accepts_plain_or_fenced_output() {
        assert_eq!(
            extract_yaml("```yaml\nschema: archon.workflow.v1\n```\n"),
            "schema: archon.workflow.v1"
        );
        assert_eq!(
            extract_yaml("schema: archon.workflow.v1\n"),
            "schema: archon.workflow.v1"
        );
    }
}
