//! Host side of `archon_workflow::llm_client_port`.
//!
//! `archon-workflow` cannot reach `archon-pipeline` (see the port's module
//! doc), so this file is where the workflow layer's LLM port meets
//! `archon_pipeline::runner::LlmClient`. It holds two adapters: one that builds
//! a client for a run, and one that forwards calls to a built client.
//!
//! Deliberately not named `workflow_*`. Every `src/command/workflow*.rs` file
//! is destined for `crates/archon-workflow`, and none of them may name
//! `archon_pipeline`; keeping the adapter outside that prefix makes the
//! invariant a one-line grep rather than a convention.

use std::sync::Arc;

use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
    ToolUseEntry,
};
use archon_tools::provider_env::ProviderEnvResolution;
use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentOutcome, WorkflowAgentToolAccess, WorkflowAgentToolUse,
    WorkflowLlmClient, WorkflowLlmClientFactory, WorkflowLlmClientRequest, WorkflowProviderEnv,
};
use async_trait::async_trait;

use crate::command::pipeline_support::build_subagent_pipeline_adapter_with_policy;

/// Presents an `archon-pipeline` client through the workflow port.
pub(crate) struct PipelineWorkflowLlmClient {
    inner: Arc<dyn LlmClient>,
    audit_provenance: Option<serde_json::Value>,
    audit_min_progress_secs: u64,
    audit_policy: Option<archon_workflow::repository_audit::budget::AuditPolicy>,
}

impl PipelineWorkflowLlmClient {
    pub(crate) fn new(inner: Arc<dyn LlmClient>) -> Self {
        Self {
            inner,
            audit_policy: None,
            audit_provenance: None,
            audit_min_progress_secs: 900,
        }
    }

    pub(crate) fn configured(
        inner: Arc<dyn LlmClient>,
        config: &ArchonConfig,
    ) -> Arc<dyn WorkflowLlmClient> {
        use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
        let resolved = config
            .workflow
            .repository_audit
            .resolve(config.workflow.generated.host_call_timeout_secs);
        let limit = |value: archon_core::config::AuditLimit| match value {
            archon_core::config::AuditLimit::Finite(n) => Limit::Finite(n),
            archon_core::config::AuditLimit::Unlimited => Limit::Unlimited,
        };
        Arc::new(Self {
            inner,
            audit_min_progress_secs: resolved.min_progress_secs.get(),
            audit_provenance: Some(
                serde_json::json!({"version":1,"sources":resolved.sources,"attempt_timeout_source":resolved.attempt_timeout_source}),
            ),
            audit_policy: Some(AuditPolicy {
                attempt_timeout_secs: limit(resolved.attempt_timeout_secs),
                total_time_secs: limit(resolved.total_time_secs),
                unexpected_change_refreshes: limit(resolved.unexpected_change_refreshes),
            }),
        })
    }

    /// The port as an owned trait object, which is how every caller wants it.
    pub(crate) fn arc(inner: Arc<dyn LlmClient>) -> Arc<dyn WorkflowLlmClient> {
        Arc::new(Self::new(inner))
    }
}

#[async_trait]
impl WorkflowLlmClient for PipelineWorkflowLlmClient {
    fn audit_min_progress_secs(&self) -> u64 {
        self.audit_min_progress_secs
    }
    fn repository_audit_provenance(&self) -> Option<serde_json::Value> {
        self.audit_provenance.clone()
    }
    fn repository_audit_policy(
        &self,
    ) -> Option<archon_workflow::repository_audit::budget::AuditPolicy> {
        self.audit_policy.clone()
    }

    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }

    fn probe_agent_read(&self, path: &std::path::Path) -> Option<Result<(), String>> {
        self.inner.probe_agent_read(path)
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.inner
            .send_message(messages, system, tools, model)
            .await
            .map(outcome_from_response)
            .map_err(WorkflowError::port)
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.inner
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await
            .map(outcome_from_response)
            .map_err(WorkflowError::port)
    }

    async fn continue_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.inner
            .continue_agent(execution_request(call)?)
            .await
            .map(outcome_from_response)
            .map_err(WorkflowError::port)
    }

    async fn run_agent(&self, call: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        let request = execution_request(call)?;
        self.inner
            .run_agent(request)
            .await
            .map(outcome_from_response)
            .map_err(WorkflowError::port)
    }
}

/// Rebuilds the pipeline's request from the port's call.
///
/// `pipeline_type` is fixed rather than carried: every call arriving here is a
/// workflow stage by construction, and a field the workflow layer could only
/// ever set one way is a field it should not have to set.
fn execution_request(call: WorkflowAgentCall) -> WorkflowResult<AgentExecutionRequest> {
    Ok(AgentExecutionRequest {
        session_id: call.session_id,
        pipeline_type: PipelineType::Workflow,
        task: call.task,
        cwd: call.cwd,
        ordinal: call.ordinal,
        attempt: call.attempt,
        agent: AgentInfo {
            key: call.agent.key,
            display_name: call.agent.display_name,
            model: call.agent.model,
            phase: call.agent.phase,
            critical: call.agent.critical,
            parallelizable: call.agent.parallelizable,
            quality_threshold: call.agent.quality_threshold,
            tool_access_level: match call.agent.tool_access {
                WorkflowAgentToolAccess::ReadOnly => ToolAccessLevel::ReadOnly,
                WorkflowAgentToolAccess::Full => ToolAccessLevel::Full,
            },
        },
        messages: call.messages,
        system: call.system,
        tools: call.tools,
        allowed_tools: call.allowed_tools,
        timeout_secs: call.timeout_secs,
        disable_auto_background: call.disable_auto_background,
        write_roots: call.write_roots,
        provider_env_resolution: call.provider_env.map(provider_env_resolution).transpose()?,
    })
}

/// Unwraps the opaque handle this crate put into the call in the first place.
///
/// Fails the call rather than dropping the environment: an agent that starts
/// without the credentials its stage declared fails later, further from the
/// cause, and after spending a provider request.
fn provider_env_resolution(env: WorkflowProviderEnv) -> WorkflowResult<ProviderEnvResolution> {
    env.downcast_ref::<ProviderEnvResolution>()
        .cloned()
        .ok_or_else(|| {
            WorkflowError::port(
                "workflow provider environment handle did not carry a host ProviderEnvResolution"
                    .to_string(),
            )
        })
}

fn outcome_from_response(response: LlmResponse) -> WorkflowAgentOutcome {
    WorkflowAgentOutcome {
        content: response.content,
        tool_uses: response.tool_uses.into_iter().map(tool_use).collect(),
        tokens_in: response.tokens_in,
        tokens_out: response.tokens_out,
        stop_reason: response.stop_reason,
    }
}

fn tool_use(entry: ToolUseEntry) -> WorkflowAgentToolUse {
    WorkflowAgentToolUse {
        tool_name: entry.tool_name,
        input: entry.input,
        output: entry.output,
    }
}

/// Owns its config rather than borrowing it, so the type carries no lifetime
/// parameter. `#[async_trait]` and lifetime-parametrised implementors interact
/// badly, and a factory outliving the borrow it was built from is the shape
/// Wave B will want anyway. The cost is one clone per CLI invocation, against a
/// run that is about to make network calls.
pub(crate) struct SubagentPipelineClientFactory {
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
    endpoint_policy: crate::command::workflow_provider_route::ProviderEndpointPolicy,
}

impl SubagentPipelineClientFactory {
    pub(crate) fn new(config: &ArchonConfig, env_vars: &ArchonEnvVars) -> Self {
        Self {
            config: config.clone(),
            env_vars: env_vars.clone(),
            endpoint_policy:
                crate::command::workflow_provider_route::ProviderEndpointPolicy::AmbientAllowed,
        }
    }

    pub(crate) fn configured_only(config: &ArchonConfig, env_vars: &ArchonEnvVars) -> Self {
        Self {
            config: config.clone(),
            env_vars: env_vars.clone(),
            endpoint_policy:
                crate::command::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
        }
    }
}

#[async_trait(?Send)]
impl WorkflowLlmClientFactory for SubagentPipelineClientFactory {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        let client = build_subagent_pipeline_adapter_with_policy(
            &self.config,
            &self.env_vars,
            &request.origin,
            &request.cwd,
            &request.session_id,
            request.read_roots,
            self.endpoint_policy,
        )
        .await
        .map_err(WorkflowError::port)?;
        Ok(PipelineWorkflowLlmClient::configured(client, &self.config))
    }
}

#[cfg(test)]
#[path = "pipeline_workflow_llm_test_support.rs"]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::{TestClientFallback, subagent_workflow_client_for_test};

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::llm_client_port::{WorkflowAgentSpec, WorkflowAgentToolAccess};

    fn call(provider_env: Option<WorkflowProviderEnv>) -> WorkflowAgentCall {
        WorkflowAgentCall {
            session_id: "run-1".into(),
            task: "stage".into(),
            cwd: None,
            ordinal: 3,
            attempt: 2,
            agent: WorkflowAgentSpec {
                key: "coder".into(),
                display_name: "Coder".into(),
                model: "sonnet".into(),
                phase: 1,
                critical: true,
                parallelizable: false,
                quality_threshold: 0.5,
                tool_access: WorkflowAgentToolAccess::Full,
            },
            messages: Vec::new(),
            system: Vec::new(),
            tools: Vec::new(),
            allowed_tools: vec!["Read".into()],
            timeout_secs: Some(17),
            disable_auto_background: true,
            write_roots: Vec::new(),
            provider_env,
        }
    }

    /// The port dropped `pipeline_type` because the workflow layer could only
    /// ever set it one way. This is the assertion that moved here with it.
    #[test]
    fn workflow_calls_stay_workflow_pipeline_type() {
        let request = execution_request(call(None)).expect("request");

        assert_eq!(request.pipeline_type, PipelineType::Workflow);
        assert_eq!(request.ordinal, 3);
        assert_eq!(request.attempt, 2);
        assert_eq!(request.timeout_secs, Some(17));
        assert!(request.disable_auto_background);
        assert!(matches!(
            request.agent.tool_access_level,
            ToolAccessLevel::Full
        ));
        assert!(request.provider_env_resolution.is_none());
    }

    /// A handle carrying something other than the host's own resolution must
    /// fail the call, not run the agent without its declared credentials.
    #[test]
    fn workflow_outcome_preserves_pipeline_stop_reason() {
        let outcome = outcome_from_response(LlmResponse {
            content: "partial".into(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 2,
            stop_reason: Some("max_tokens".into()),
        });

        assert_eq!(outcome.stop_reason.as_deref(), Some("max_tokens"));
    }

    #[test]
    fn foreign_provider_environment_handle_fails_the_call() {
        let error = execution_request(call(Some(WorkflowProviderEnv::new(7u32))))
            .expect_err("foreign handle must not be silently dropped");

        assert!(
            error.to_string().contains("ProviderEnvResolution"),
            "unexpected error: {error}"
        );
    }
}

#[cfg(test)]
#[path = "acceptance_scratch_guardian_tests.rs"]
mod native_policy_tests;
