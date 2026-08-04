//! Real `archon-pipeline` clients, assembled for host-wiring tests.
//!
//! Three test suites — provider usage ledgers, on-the-wire request shape, and
//! context-compaction recovery — assert on the concrete
//! `SubagentPipelineClient`, because that wiring is what they exist to check.
//! They then drive it through workflow code that only knows
//! [`WorkflowLlmClient`]. Assembling the pair here is what keeps
//! `archon_pipeline` out of `src/command/workflow*.rs` without weakening any of
//! them.

use std::path::PathBuf;
use std::sync::Arc;

use archon_llm::provider::LlmProvider;
use archon_pipeline::llm_adapter::ProviderLlmAdapter;
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
};
use archon_pipeline::subagent_adapter::SubagentPipelineClient;
use archon_tools::tool::ToolContext;
use archon_workflow::llm_client_port::WorkflowLlmClient;
use async_trait::async_trait;

use super::PipelineWorkflowLlmClient;

/// What the assembled client does when the subagent path hands work back.
pub(crate) enum TestClientFallback<'a> {
    /// Ordinary provider completions, the production shape.
    Provider,
    /// Provider completions pinned to one session id. A test measuring
    /// per-session provider usage needs every call attributed to the scope it
    /// is counting, or it silently undercounts its own traffic.
    ProviderScopedTo(&'a str),
    /// Reaching the fallback fails the call. This is how a test asserts that
    /// the real subagent path ran rather than quietly degrading to a
    /// completion that would still look like a pass.
    Forbidden,
}

pub(crate) fn subagent_workflow_client_for_test(
    provider: Arc<dyn LlmProvider>,
    origin: &str,
    working_dir: PathBuf,
    fallback: TestClientFallback<'_>,
) -> Arc<dyn WorkflowLlmClient> {
    let provider_fallback = || -> Arc<dyn LlmClient> {
        Arc::new(ProviderLlmAdapter::new(Arc::clone(&provider)).with_origin(origin.to_string()))
    };
    let fallback: Arc<dyn LlmClient> = match fallback {
        TestClientFallback::Provider => provider_fallback(),
        TestClientFallback::ProviderScopedTo(session_id) => Arc::new(SessionScopedClient {
            inner: provider_fallback(),
            session_id: session_id.to_string(),
        }),
        TestClientFallback::Forbidden => Arc::new(ForbiddenFallbackClient),
    };
    PipelineWorkflowLlmClient::arc(Arc::new(SubagentPipelineClient::with_provider(
        fallback,
        ToolContext {
            working_dir,
            ..ToolContext::default()
        },
        provider,
    )))
}

/// Forces one session id onto everything reaching the provider.
///
/// Plain completions are promoted to agent calls so they are ledgered the same
/// way subagent calls are; a usage measurement that counted only some of its
/// own calls would understate by an amount nobody could see.
struct SessionScopedClient {
    inner: Arc<dyn LlmClient>,
    session_id: String,
}

#[async_trait]
impl LlmClient for SessionScopedClient {
    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> anyhow::Result<LlmResponse> {
        self.inner
            .run_agent(AgentExecutionRequest {
                session_id: self.session_id.clone(),
                pipeline_type: PipelineType::Workflow,
                task: "controlled canary planner call".into(),
                cwd: None,
                ordinal: 0,
                attempt: 1,
                agent: AgentInfo {
                    key: "planner".into(),
                    display_name: "Planner".into(),
                    model: model.into(),
                    phase: 0,
                    critical: true,
                    parallelizable: false,
                    quality_threshold: 0.0,
                    tool_access_level: ToolAccessLevel::ReadOnly,
                },
                messages,
                system,
                tools,
                allowed_tools: Vec::new(),
                timeout_secs: None,
                disable_auto_background: true,
                provider_env_resolution: None,
            })
            .await
    }

    async fn run_agent(&self, mut request: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        request.session_id = self.session_id.clone();
        self.inner.run_agent(request).await
    }
}

struct ForbiddenFallbackClient;

#[async_trait]
impl LlmClient for ForbiddenFallbackClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("real subagent path must not use fallback")
    }
}
