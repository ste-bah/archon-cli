use std::sync::Arc;

use archon_tools::board::DelegatedOutcome;
use archon_tools::provider_env::ProviderEnvResolution;
use archon_workflow::{
    ProviderTier, SharedWorkflowUiSink, StageKind, StageRunRequest, WorkflowActivityStatus,
    WorkflowActivityUpdate, WorkflowAgentCall, WorkflowLlmClient, WorkflowProviderEnv,
    WorkflowUiEvent, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
    WorkflowV2HostMethod, WorkflowV2WriteMode,
};

use archon_workflow::v2::project_artifact_stamping::stamp_project_artifact_paths;

use archon_workflow::llm_retry::run_agent_with_transient_retry;

use super::super::workflow_live_runner::workflow_live_stage_board::{
    StageBoardItem, stage_board_outcome,
};
use super::super::workflow_live_runner::{
    allowed_tools, tier_model_alias, workflow_agent, workflow_agent_ordinal,
    workflow_agent_session_id,
};
use archon_workflow::stage_activity::{activity_detail, request_target_repository_root};

pub(crate) const EXACT_TOOL_POLICY_MARKER: &str = "__ARCHON_EXACT_TOOLS__";

#[derive(Clone)]
pub(super) struct LiveV2AgentClient {
    llm: Arc<dyn WorkflowLlmClient>,
    pub(super) ui_sink: SharedWorkflowUiSink,
    provider_tier: ProviderTier,
    agent_names: Vec<String>,
    run_id: String,
    target_repository_root: Option<String>,
    timeout_secs: Option<u64>,
    provider_env_resolution: Option<ProviderEnvResolution>,
    fixed_raw_tool_policy: Option<Vec<String>>,
    pub(super) audit: Option<archon_workflow::repository_audit::runtime::AuditRuntime>,
}

impl LiveV2AgentClient {
    pub(super) fn new(
        llm: Arc<dyn WorkflowLlmClient>,
        ui_sink: SharedWorkflowUiSink,
        agent_names: Vec<String>,
        run_id: String,
        target_repository_root: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            llm,
            ui_sink,
            provider_tier: ProviderTier::Researcher,
            agent_names,
            run_id,
            target_repository_root,
            timeout_secs,
            provider_env_resolution: None,
            fixed_raw_tool_policy: None,
            audit: None,
        }
    }

    pub(super) fn audit_provenance(&self) -> Option<serde_json::Value> { self.llm.repository_audit_provenance() }
    pub(super) fn audit_policy(&self) -> Option<archon_workflow::repository_audit::budget::AuditPolicy> {
        self.llm.repository_audit_policy()
    }
    pub(super) fn with_audit(&self, audit: archon_workflow::repository_audit::runtime::AuditRuntime) -> Self {
        let mut client = self.clone(); client.audit = Some(audit); client
    }
    pub(super) fn for_audit(&self) -> Self {
        let mut client = self.with_provider_tier(ProviderTier::Critic);
        client.audit = None;
        client.fixed_raw_tool_policy = Some(vec!["Read".into(), "Grep".into(), "Glob".into()]);
        client
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

    pub(super) fn with_fixed_raw_tool_policy(mut self, tools: Vec<String>) -> Self {
        self.fixed_raw_tool_policy = Some(tools);
        self
    }

    pub(super) fn with_provider_tier(&self, provider_tier: ProviderTier) -> Self {
        Self {
            llm: self.llm.clone(),
            ui_sink: self.ui_sink.clone(),
            provider_tier,
            agent_names: self.agent_names.clone(),
            run_id: self.run_id.clone(),
            target_repository_root: self.target_repository_root.clone(),
            timeout_secs: self.timeout_secs,
            provider_env_resolution: self.provider_env_resolution.clone(),
            fixed_raw_tool_policy: self.fixed_raw_tool_policy.clone(),
            audit: self.audit.clone(),
        }
    }

    /// The per-dispatch timeout this client applies, if one is configured.
    pub(super) fn timeout_secs(&self) -> Option<u64> {
        self.timeout_secs
    }

    pub(super) fn with_timeout_secs(&self, timeout_secs: Option<u64>) -> Self {
        Self {
            llm: self.llm.clone(),
            ui_sink: self.ui_sink.clone(),
            provider_tier: self.provider_tier,
            agent_names: self.agent_names.clone(),
            run_id: self.run_id.clone(),
            target_repository_root: self.target_repository_root.clone(),
            timeout_secs,
            provider_env_resolution: self.provider_env_resolution.clone(),
            fixed_raw_tool_policy: self.fixed_raw_tool_policy.clone(),
            audit: self.audit.clone(),
        }
    }

    pub(super) fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        read_only_v2_fanout_parallelism(requested, live_v2_subagent_max_concurrency())
    }

    pub(super) fn read_only_fanout_parallelism(&self, requested: Option<usize>) -> usize {
        self.fanout_parallelism(requested)
    }

    pub(super) async fn run_agent_raw_request(
        &self,
        request: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> std::result::Result<archon_workflow::WorkflowAgentOutcome, WorkflowV2AgentError> {
        let tools = self.fixed_raw_tool_policy.as_ref().ok_or_else(|| {
            WorkflowV2AgentError::Transport(
                "fixed raw outcome call has no host-owned tool policy".to_string(),
            )
        })?;
        if tools.is_empty() {
            return Err(WorkflowV2AgentError::Transport(
                "fixed raw outcome tool policy is empty".to_string(),
            ));
        }
        let stage_request = stage_request_for_v2_agent(
            &self.run_id,
            self.provider_tier,
            self.target_repository_root.clone(),
            request,
        );
        let model_alias = tier_model_alias(self.provider_tier).to_string();
        let agent = workflow_agent(&stage_request, &model_alias, &self.agent_names);
        let session_id = workflow_agent_session_id(&stage_request);
        let ordinal = workflow_agent_ordinal(&stage_request);
        let mut allowed_tools = Vec::with_capacity(tools.len() + 1);
        allowed_tools.push(EXACT_TOOL_POLICY_MARKER.to_string());
        allowed_tools.extend(tools.iter().cloned());
        let call = WorkflowAgentCall {
            session_id,
            task: request.task.clone(),
            cwd: request_target_repository_root(&stage_request),
            ordinal,
            attempt: stage_request.attempt as usize,
            agent,
            messages: vec![serde_json::json!({ "role": "user", "content": prompt })],
            system: Vec::new(),
            tools: Vec::new(),
            allowed_tools,
            timeout_secs: self.timeout_secs,
            disable_auto_background: true,
            write_roots: Vec::new(),
            provider_env: self
                .provider_env_resolution
                .clone()
                .map(WorkflowProviderEnv::new),
        };
        run_agent_with_transient_retry(&self.llm, call, |_attempt| async { Ok(()) })
            .await
            .map_err(|error| WorkflowV2AgentError::Transport(error.to_string()))
    }

    fn activity_event(
        request: &StageRunRequest,
        agent_name: &str,
        provider_id: &str,
        model: &str,
        status: WorkflowActivityStatus,
        detail: &str,
    ) -> WorkflowUiEvent {
        WorkflowUiEvent::Activity(WorkflowActivityUpdate {
            id: format!("workflow:{}:{}", request.run_id, request.stage_id),
            name: agent_name.to_string(),
            status,
            detail: Some(activity_detail(request, detail)),
            run_id: Some(request.run_id.clone()),
            provider: Some(provider_id.to_string()),
            model: Some(model.to_string()),
        })
    }

    async fn emit_required_activity(
        &self,
        request: &StageRunRequest,
        agent_name: &str,
        provider_id: &str,
        model: &str,
        status: WorkflowActivityStatus,
        detail: &str,
    ) -> std::result::Result<(), WorkflowV2AgentError> {
        self.ui_sink
            .emit(Self::activity_event(
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

/// One definition, shared with the shape tuner.
///
/// The learner's baseline has to be the cap this function returns, or a
/// "narrowing" could be reported against a number the runtime never used. Two
/// copies of this resolution is how that happens, so there is one.
fn live_v2_subagent_max_concurrency() -> Option<usize> {
    crate::command::sona_workflow_shape_tuning::resolved_subagent_cap()
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
            WorkflowActivityStatus::Running,
            "v2 call running",
        )
        .await?;
        // The V2 lifecycle is the other dispatch path — it reuses this file's
        // helpers but never enters `PipelineWorkflowRunner::run_stage` — so it
        // has to raise its own branch onto the board or a decomposed run, which
        // is most of what a real run does, stays invisible (#161).
        let session_id = workflow_agent_session_id(&stage_request);
        let ordinal = workflow_agent_ordinal(&stage_request);
        let mut board =
            StageBoardItem::raise(&stage_request, &session_id, ordinal, &agent_name, &prompt);
        let prompt_parts =
            archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(request);
        let agent_request = WorkflowAgentCall {
            session_id,
            task: request.task.clone(),
            cwd: request_target_repository_root(&stage_request),
            ordinal,
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
            allowed_tools: if let Some(tools) = &self.fixed_raw_tool_policy {
                std::iter::once(EXACT_TOOL_POLICY_MARKER.to_string()).chain(tools.iter().cloned()).collect()
            } else { allowed_tools(&stage_request) },
            timeout_secs: self.timeout_secs,
            disable_auto_background: true,
            // Resolved here because here is where both halves are in scope: the
            // artifact context the host built for this call, and the repository
            // root the call itself targets. The v2 lifecycle already had them —
            // it was serialising them into the prompt a few lines below — and
            // dropping them at the port is what left write confinement with
            // nothing to enforce.
            write_roots: archon_workflow::v2::project_artifact_write_roots::declared_write_roots(
                &request.project_artifacts,
                request.repository_root.as_deref(),
            ),
            // Wrapped, not read: the port carries the host's resolution back to
            // the host adapter without this layer or `archon-workflow` ever
            // seeing the credential values inside it.
            provider_env: self
                .provider_env_resolution
                .clone()
                .map(WorkflowProviderEnv::new),
        };
        let response = match run_agent_with_transient_retry(&self.llm, agent_request, |attempt| {
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
                        WorkflowActivityStatus::Running,
                        &format!("v2 call retrying after transient provider error ({attempt}/3)"),
                    )
                    .await
                    .map_err(|error| {
                        archon_workflow::WorkflowError::NotificationDelivery(error.to_string())
                    })
            }
        })
        .await
        {
            Ok(response) => response,
            Err(err) => {
                // Before the emit below, which is itself a `?`.
                board.finish(stage_board_outcome(&err));
                let notification_delivery = matches!(
                    &err,
                    archon_workflow::WorkflowError::NotificationDelivery(_)
                );
                self.emit_required_activity(
                    &stage_request,
                    &agent_name,
                    &provider_id,
                    &resolved_model,
                    WorkflowActivityStatus::Failed,
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
            WorkflowActivityStatus::Complete,
            "v2 call complete",
        )
        .await?;
        // `in_review`, not `resolved`: the branch returned, nothing verified it.
        board.finish(DelegatedOutcome::Completed);
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

#[path = "workflow_live_v2_client_context.rs"]
mod context;
use context::*;

#[cfg(test)]
#[path = "workflow_live_v2_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "workflow_live_v2_client_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_live_v2_wire_tests.rs"]
mod wire_tests;
