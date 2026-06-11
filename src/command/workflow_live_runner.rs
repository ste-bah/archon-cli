use std::path::PathBuf;
use std::sync::Arc;

use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, PipelineType, ToolAccessLevel,
};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::{
    ProviderTier, StageKind, StageRunOutput, StageRunRequest, WorkflowStageRunner,
};

use super::workflow_agent_select::select_workflow_agent_key;
use super::workflow_live_prompt::workflow_prompt;
use super::workflow_live_retry;

pub(crate) struct PipelineWorkflowRunner {
    pub(crate) llm: Arc<dyn LlmClient>,
    pub(crate) tui_tx: TuiEventSender,
    pub(crate) agent_names: Vec<String>,
}

#[async_trait::async_trait]
impl WorkflowStageRunner for PipelineWorkflowRunner {
    fn max_concurrency(&self) -> Option<usize> {
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
        let agent = workflow_agent(&request, &model_alias, &self.agent_names);
        let agent_name = agent.key.clone();
        self.emit_activity(
            &request,
            &agent_name,
            &provider_id,
            &resolved_model,
            AgentActivityStatus::Running,
            "stage running",
        );
        let agent_request = AgentExecutionRequest {
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
        };
        let response = match workflow_live_retry::run_agent_with_transient_retry(
            &self.llm,
            agent_request,
            |attempt| {
                self.emit_activity(
                    &request,
                    &agent_name,
                    &provider_id,
                    &resolved_model,
                    AgentActivityStatus::Running,
                    &format!("stage retrying after transient provider error ({attempt}/3)"),
                );
            },
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                self.emit_activity(
                    &request,
                    &agent_name,
                    &provider_id,
                    &resolved_model,
                    AgentActivityStatus::Failed,
                    "stage failed",
                );
                return Err(err);
            }
        };
        self.emit_activity(
            &request,
            &agent_name,
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

pub(crate) fn request_target_repository_root(request: &StageRunRequest) -> Option<PathBuf> {
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
        agent_name: &str,
        provider_id: &str,
        model: &str,
        status: AgentActivityStatus,
        detail: &str,
    ) {
        let _ = self
            .tui_tx
            .send(TuiEvent::AgentActivity(AgentActivityUpdate {
                id: format!("workflow:{}:{}", request.run_id, request.stage_id),
                name: agent_name.to_string(),
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

fn workflow_agent(request: &StageRunRequest, model: &str, agent_names: &[String]) -> AgentInfo {
    let key = select_workflow_agent_key(request, agent_names);
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

pub(crate) fn allowed_tools(request: &StageRunRequest) -> Vec<String> {
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
    if command_execution_stage_id(&request.stage_id) {
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
    command_execution_text(&haystack)
}

fn command_execution_stage_id(stage_id: &str) -> bool {
    let id = stage_id.to_ascii_lowercase().replace('-', "_");
    id.ends_with("_tests")
        || id.contains("_post_tests")
        || id.contains("_focused_tests")
        || id.contains("_verification")
}

fn command_execution_text(haystack: &str) -> bool {
    [
        "focused_test",
        "focused-test",
        "focused test",
        "focused tests",
        "post-remediation tests",
        "post remediation tests",
        "cargo test",
        "test command",
        "test execution",
        "test evidence",
        "run tests",
        "run focused",
        "tests and checks",
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

pub(crate) fn extract_yaml(content: &str) -> String {
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

pub(crate) fn tier_model_alias(tier: ProviderTier) -> &'static str {
    match tier {
        ProviderTier::Cheap | ProviderTier::Local => "haiku",
        ProviderTier::Critic | ProviderTier::Reducer => "opus",
        ProviderTier::Planner
        | ProviderTier::Researcher
        | ProviderTier::Coder
        | ProviderTier::Vision => "sonnet",
    }
}
