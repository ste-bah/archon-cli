use std::sync::Arc;

use archon_tools::provider_env::ProviderEnvResolution;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::{
    ProviderTier, StageKind, StageRunRequest, WorkflowAgentCall, WorkflowLlmClient,
    WorkflowProviderEnv, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
    WorkflowV2HostMethod, WorkflowV2WriteMode,
};

use super::workflow_live_v2_artifact_paths::stamp_project_artifact_paths;

use super::super::workflow_live_retry;
use super::super::workflow_live_runner::{
    allowed_tools, request_target_repository_root, tier_model_alias, workflow_agent,
    workflow_agent_ordinal, workflow_agent_session_id,
};
use super::super::workflow_live_runner_activity::activity_detail;

#[derive(Clone)]
pub(super) struct LiveV2AgentClient {
    llm: Arc<dyn WorkflowLlmClient>,
    pub(super) tui_tx: TuiEventSender,
    provider_tier: ProviderTier,
    agent_names: Vec<String>,
    run_id: String,
    target_repository_root: Option<String>,
    timeout_secs: Option<u64>,
    provider_env_resolution: Option<ProviderEnvResolution>,
}

impl LiveV2AgentClient {
    pub(super) fn new(
        llm: Arc<dyn WorkflowLlmClient>,
        tui_tx: TuiEventSender,
        agent_names: Vec<String>,
        run_id: String,
        target_repository_root: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            llm,
            tui_tx,
            provider_tier: ProviderTier::Researcher,
            agent_names,
            run_id,
            target_repository_root,
            timeout_secs,
            provider_env_resolution: None,
        }
    }

    pub(super) fn with_provider_env_resolution(
        mut self,
        provider_env_resolution: Option<ProviderEnvResolution>,
    ) -> Self {
        self.provider_env_resolution = provider_env_resolution;
        self
    }

    pub(super) fn provider_env_resolution(&self) -> Option<&ProviderEnvResolution> {
        self.provider_env_resolution.as_ref()
    }

    pub(super) fn with_provider_tier(&self, provider_tier: ProviderTier) -> Self {
        Self {
            llm: self.llm.clone(),
            tui_tx: self.tui_tx.clone(),
            provider_tier,
            agent_names: self.agent_names.clone(),
            run_id: self.run_id.clone(),
            target_repository_root: self.target_repository_root.clone(),
            timeout_secs: self.timeout_secs,
            provider_env_resolution: self.provider_env_resolution.clone(),
        }
    }

    pub(super) fn with_timeout_secs(&self, timeout_secs: Option<u64>) -> Self {
        Self {
            llm: self.llm.clone(),
            tui_tx: self.tui_tx.clone(),
            provider_tier: self.provider_tier,
            agent_names: self.agent_names.clone(),
            run_id: self.run_id.clone(),
            target_repository_root: self.target_repository_root.clone(),
            timeout_secs,
            provider_env_resolution: self.provider_env_resolution.clone(),
        }
    }

    pub(super) fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        read_only_v2_fanout_parallelism(requested, live_v2_subagent_max_concurrency())
    }

    pub(super) fn read_only_fanout_parallelism(&self, requested: Option<usize>) -> usize {
        self.fanout_parallelism(requested)
    }

    fn activity_event(
        request: &StageRunRequest,
        agent_name: &str,
        provider_id: &str,
        model: &str,
        status: AgentActivityStatus,
        detail: &str,
    ) -> TuiEvent {
        TuiEvent::AgentActivity(AgentActivityUpdate {
            id: format!("workflow:{}:{}", request.run_id, request.stage_id),
            name: agent_name.to_string(),
            role: AgentActivityRole::Subagent,
            status,
            current_tool: None,
            detail: Some(activity_detail(request, detail)),
            run_id: Some(request.run_id.clone()),
            parent_id: None,
            artifact_id: None,
            provider: Some(provider_id.to_string()),
            model: Some(model.to_string()),
            cost_usd: None,
        })
    }

    async fn emit_required_activity(
        &self,
        request: &StageRunRequest,
        agent_name: &str,
        provider_id: &str,
        model: &str,
        status: AgentActivityStatus,
        detail: &str,
    ) -> std::result::Result<(), WorkflowV2AgentError> {
        self.tui_tx
            .send_async(Self::activity_event(
                request,
                agent_name,
                provider_id,
                model,
                status,
                detail,
            ))
            .await
            .map_err(|error| {
                WorkflowV2AgentError::NotificationDelivery(format!(
                    "workflow V2 agent activity delivery failed: run_id={} stage_id={} status={status:?} provider={} model={}: {error}",
                    request.run_id, request.stage_id, provider_id, model
                ))
            })
    }
}

fn live_v2_subagent_max_concurrency() -> Option<usize> {
    archon_tools::subagent_executor::get_subagent_executor()
        .and_then(|exec| exec.max_concurrency())
        .or(Some(
            archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT,
        ))
}

fn read_only_v2_fanout_parallelism(requested: Option<usize>, subagent_cap: Option<usize>) -> usize {
    let cap = subagent_cap
        .unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT)
        .max(1);
    requested.map_or(cap, |requested| requested.max(1).min(cap))
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for LiveV2AgentClient {
    async fn run_agent_request(
        &self,
        request: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> std::result::Result<String, WorkflowV2AgentError> {
        let stage_request = stage_request_for_v2_agent(
            &self.run_id,
            self.provider_tier,
            self.target_repository_root.clone(),
            request,
        );
        let model_alias = tier_model_alias(self.provider_tier).to_string();
        let resolved_model = self.llm.resolve_model_alias(&model_alias);
        let provider_id = self
            .llm
            .provider_id()
            .unwrap_or_else(|| "active-provider".to_string());
        let agent = workflow_agent(&stage_request, &model_alias, &self.agent_names);
        let agent_name = agent.key.clone();
        self.emit_required_activity(
            &stage_request,
            &agent_name,
            &provider_id,
            &resolved_model,
            AgentActivityStatus::Running,
            "v2 call running",
        )
        .await?;
        let prompt_parts =
            archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(request);
        let agent_request = WorkflowAgentCall {
            session_id: workflow_agent_session_id(&stage_request),
            task: request.task.clone(),
            cwd: request_target_repository_root(&stage_request),
            ordinal: workflow_agent_ordinal(&stage_request),
            attempt: stage_request.attempt as usize,
            agent,
            messages: vec![serde_json::json!({
                "role": "user",
                "content": prompt,
            })],
            system: vec![serde_json::json!({
                "type": "text",
                "text": format!(
                    "{}\n\n{}",
                    v2_system_context(),
                    prompt_parts.stable_prefix
                ),
            })],
            tools: super::workflow_live_provider_env::provider_env_tool_markers(request),
            allowed_tools: allowed_tools(&stage_request),
            timeout_secs: self.timeout_secs,
            disable_auto_background: true,
            // Wrapped, not read: the port carries the host's resolution back to
            // the host adapter without this layer or `archon-workflow` ever
            // seeing the credential values inside it.
            provider_env: self
                .provider_env_resolution
                .clone()
                .map(WorkflowProviderEnv::new),
        };
        let response = match workflow_live_retry::run_agent_with_transient_retry(
            &self.llm,
            agent_request,
            |attempt| {
                let client = self.clone();
                let stage_request = stage_request.clone();
                let agent_name = agent_name.clone();
                let provider_id = provider_id.clone();
                let resolved_model = resolved_model.clone();
                async move {
                    client
                        .emit_required_activity(
                            &stage_request,
                            &agent_name,
                            &provider_id,
                            &resolved_model,
                            AgentActivityStatus::Running,
                            &format!(
                                "v2 call retrying after transient provider error ({attempt}/3)"
                            ),
                        )
                        .await
                        .map_err(|error| {
                            archon_workflow::WorkflowError::NotificationDelivery(error.to_string())
                        })
                }
            },
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                let notification_delivery = matches!(
                    &err,
                    archon_workflow::WorkflowError::NotificationDelivery(_)
                );
                self.emit_required_activity(
                    &stage_request,
                    &agent_name,
                    &provider_id,
                    &resolved_model,
                    AgentActivityStatus::Failed,
                    "v2 call failed",
                )
                .await?;
                if notification_delivery {
                    return Err(WorkflowV2AgentError::NotificationDelivery(err.to_string()));
                }
                return Err(WorkflowV2AgentError::Transport(err.to_string()));
            }
        };
        self.emit_required_activity(
            &stage_request,
            &agent_name,
            &provider_id,
            &resolved_model,
            AgentActivityStatus::Complete,
            "v2 call complete",
        )
        .await?;
        Ok(response.content)
    }

    async fn run_agent(&self, prompt: String) -> std::result::Result<String, WorkflowV2AgentError> {
        let request = WorkflowV2AgentRequest {
            call: archon_workflow::WorkflowV2HostCall {
                id: "v2-agent".to_string(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: Default::default(),
            },
            role: "researcher".to_string(),
            task: prompt.clone(),
            constraints: Vec::new(),
            input: serde_json::Value::Null,
            repository_root: self.target_repository_root.clone(),
            project_artifacts: Default::default(),
            target_files: Vec::new(),
            target_ownership_scopes: Vec::new(),
        };
        self.run_agent_request(&request, prompt).await
    }
}

fn stage_request_for_v2_agent(
    run_id: &str,
    provider_tier: ProviderTier,
    default_repository_root: Option<String>,
    request: &WorkflowV2AgentRequest,
) -> StageRunRequest {
    StageRunRequest {
        run_id: run_id.to_string(),
        stage_id: request.call.id.clone(),
        stage_kind: stage_kind_for_v2_agent(request),
        agent: Some(request.role.clone()),
        task: request.task.clone(),
        attempt: 1,
        provider_tier,
        depends_on: Vec::new(),
        input: stage_input_for_v2_agent(default_repository_root, request),
    }
}

fn stage_input_for_v2_agent(
    default_repository_root: Option<String>,
    request: &WorkflowV2AgentRequest,
) -> serde_json::Value {
    let mut input = match request.input.clone() {
        serde_json::Value::Object(object) => serde_json::Value::Object(object),
        value => serde_json::json!({ "input": value }),
    };
    if let Some(object) = input.as_object_mut() {
        if let Some(root) = request
            .repository_root
            .clone()
            .or(default_repository_root)
            .filter(|root| !root.trim().is_empty())
        {
            object.insert(
                "target_repository_root".to_string(),
                serde_json::Value::String(root),
            );
        }
        insert_project_artifact_context(object, request);
        object.insert(
            "stage_task".to_string(),
            serde_json::Value::String(request.task.clone()),
        );
        object.insert(
            "v2_call".to_string(),
            serde_json::json!({
                "id": request.call.id,
                "method": request.call.method.as_str(),
                "role": request.role,
                "write_mode": request.call.write_mode,
                "target_files": request.target_files,
            }),
        );
        if request.call.write_mode.is_some() {
            object.insert(
                "write_coordination".to_string(),
                serde_json::json!({
                    "enabled": matches!(
                        request.call.write_mode,
                        Some(WorkflowV2WriteMode::Coordinated | WorkflowV2WriteMode::Worktree)
                    ),
                    "mode": request.call.write_mode,
                    "target_files": request.target_files,
                }),
            );
        }
    }
    input
}

fn insert_project_artifact_context(
    object: &mut serde_json::Map<String, serde_json::Value>,
    request: &WorkflowV2AgentRequest,
) {
    if request.project_artifacts.is_empty() {
        return;
    }
    if let Some(root) = request.project_artifacts.project_root.clone() {
        object.insert(
            "project_artifact_root".to_string(),
            serde_json::Value::String(root),
        );
    }
    object.insert(
        "project_artifact_roots".to_string(),
        serde_json::json!(request.project_artifacts.artifact_roots),
    );
    if let Some(root) = request.project_artifacts.branch_evidence_root.clone() {
        object.insert(
            "workflow_branch_evidence_root".to_string(),
            serde_json::Value::String(root),
        );
    }
    let resolved = request
        .project_artifacts
        .project_root
        .as_deref()
        .map(|root| stamp_project_artifact_paths(object, root))
        .unwrap_or_default();
    if !resolved.is_empty() {
        object.insert(
            "project_artifact_paths".to_string(),
            serde_json::json!(resolved),
        );
    }
    if request.is_write_capable() {
        mark_required_bash(object);
    }
}

fn mark_required_bash(object: &mut serde_json::Map<String, serde_json::Value>) {
    let extra = object
        .entry("stage_extra".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(extra) = extra.as_object_mut() else {
        return;
    };
    let tools = extra
        .entry("required_tools".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let Some(tools) = tools.as_array_mut() else {
        *tools = serde_json::json!(["Bash"]);
        return;
    };
    if !tools.iter().any(|tool| tool.as_str() == Some("Bash")) {
        tools.push(serde_json::json!("Bash"));
    }
}

fn stage_kind_for_v2_agent(request: &WorkflowV2AgentRequest) -> StageKind {
    if request.is_write_capable() {
        return StageKind::Implementation;
    }
    match request.call.method {
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => StageKind::Reduce,
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => {
            StageKind::QualityGate
        }
        WorkflowV2HostMethod::Checkpoint => StageKind::Checkpoint,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => StageKind::Tool,
        WorkflowV2HostMethod::Implementation => StageKind::Implementation,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => StageKind::Fanout,
        WorkflowV2HostMethod::Agent => StageKind::Agent,
    }
}

fn v2_system_context() -> &'static str {
    "You are an Archon dynamic workflow stage agent. Return exactly one JSON object matching the Workflow V2 result envelope from the user message."
}

#[cfg(test)]
#[path = "workflow_live_v2_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "workflow_live_v2_client_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_live_v2_wire_tests.rs"]
mod wire_tests;
