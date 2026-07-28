//! Agent-backed pipeline adapter.
//!
//! The pipeline runner is provider-neutral: tests and CLI paths can keep using
//! raw [`LlmClient::send_message`], while interactive sessions can wrap the
//! same client with this adapter so each pipeline stage runs as a real Archon
//! subagent with tools, memory, transcripts, and activity events.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_tools::agent_tool::{
    SubagentRequest, run_subagent_foreground_with_system, run_subagent_with_system,
};
use archon_tools::provider_env::{
    ProviderEnvPolicy, ProviderEnvSource, provider_env_policy_from_marker,
};
use archon_tools::subagent_executor::SubagentOutcome;
use archon_tools::tool::ToolContext;

use crate::runner::{AgentExecutionRequest, LlmClient, LlmResponse, PipelineType, ToolAccessLevel};

const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "WebSearch",
    "WebFetch",
    "DocList",
    "DocGet",
    "DocStatus",
    "DocSearch",
    "DocAnswer",
    "DocProvenance",
    "DocInspect",
    "DocModelStatus",
    "memory_recall",
    "LeannSearch",
    "LeannFindSimilar",
    "lsp",
    "CartographerScan",
    "ToolSearch",
    "AgentCatalog",
];

const FULL_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "ApplyPatch",
    "Bash",
    "Grep",
    "Glob",
    "WebSearch",
    "WebFetch",
    "DocIngest",
    "DocList",
    "DocGet",
    "DocStatus",
    "DocSearch",
    "DocAnswer",
    "DocProvenance",
    "DocInspect",
    "DocModelStatus",
    "memory_store",
    "memory_recall",
    "LeannSearch",
    "LeannFindSimilar",
    "lsp",
    "CartographerScan",
    "ToolSearch",
    "AgentCatalog",
    "TodoWrite",
];

struct SubagentPipelinePrompt {
    prompt: String,
    system: Vec<serde_json::Value>,
}

pub struct SubagentPipelineClient {
    fallback: Arc<dyn LlmClient>,
    context: ToolContext,
    activity_provider: Option<Arc<dyn LlmProvider>>,
}

impl SubagentPipelineClient {
    pub fn new(fallback: Arc<dyn LlmClient>, context: ToolContext) -> Self {
        Self {
            fallback,
            context,
            activity_provider: None,
        }
    }

    pub fn with_provider(
        fallback: Arc<dyn LlmClient>,
        context: ToolContext,
        provider: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            fallback,
            context,
            activity_provider: Some(provider),
        }
    }

    fn allowed_tools(request: &AgentExecutionRequest) -> Vec<String> {
        if !request.allowed_tools.is_empty() {
            return request.allowed_tools.clone();
        }

        let source: &[&str] = match request.agent.tool_access_level {
            ToolAccessLevel::ReadOnly => READ_ONLY_TOOLS,
            ToolAccessLevel::Full => FULL_TOOLS,
        };
        source.iter().map(|tool| (*tool).to_string()).collect()
    }

    fn prompt_for_request(request: &AgentExecutionRequest) -> SubagentPipelinePrompt {
        let mut parts = vec![format!(
            "## Pipeline Agent Run\nPipeline: {:?}\nSession: {}\nAgent: {} ({})\nPhase: {}\nOrdinal: {}\nAttempt: {}\n\n## Pipeline Task\n{}",
            request.pipeline_type,
            request.session_id,
            request.agent.key,
            request.agent.display_name,
            request.agent.phase,
            request.ordinal,
            request.attempt,
            request.task
        )];

        parts.push(format!(
            "## Archon Tool Contract\nUse only these Archon tool names for this run: {}.\nDo not call legacy MCP, Claude Flow, God pipeline, or ruv-swarm tool names even if old imported agent text mentions them. Do not run `claude-flow` or `npx ruv-swarm` through Bash. Map code search to LeannSearch/lsp/Grep/Read, memory work to memory_recall/memory_store, research/doc work to Doc*/WebSearch/WebFetch, and delegation to Agent.",
            Self::allowed_tools(request).join(", ")
        ));

        let message_text = values_to_text(&request.messages);
        if !message_text.trim().is_empty() {
            parts.push(format!("## Agent Prompt\n{message_text}"));
        }

        SubagentPipelinePrompt {
            prompt: parts.join("\n\n"),
            system: request.system.clone(),
        }
    }

    fn activity_model(&self, requested: &str) -> String {
        let Some(provider) = &self.activity_provider else {
            return requested.to_string();
        };
        let mut request = LlmRequest {
            model: requested.to_string(),
            ..LlmRequest::default()
        };
        provider.resolve_request_model(&mut request);
        request.model
    }

    fn cwd_for_request(&self, request: &AgentExecutionRequest) -> String {
        request
            .cwd
            .as_ref()
            .unwrap_or(&self.context.working_dir)
            .display()
            .to_string()
    }

    fn strict_workspace_boundary(
        request: &AgentExecutionRequest,
        allowed_tools: &[String],
    ) -> bool {
        request.pipeline_type == PipelineType::Workflow
            && request.agent.tool_access_level == ToolAccessLevel::Full
            && request.cwd.is_some()
            && !allowed_tools
                .iter()
                .any(|tool| tool.eq_ignore_ascii_case("Bash"))
    }
}

#[async_trait]
impl LlmClient for SubagentPipelineClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse> {
        self.fallback
            .send_message(messages, system, tools, model)
            .await
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        let prompt = Self::prompt_for_request(&request);
        let activity_model = self.activity_model(&request.agent.model);
        let allowed_tools = Self::allowed_tools(&request);
        let strict_workspace_boundary = Self::strict_workspace_boundary(&request, &allowed_tools);
        let provider_env = workflow_provider_env_source(&request);
        let system = prompt.system;
        let req = SubagentRequest {
            prompt: prompt.prompt,
            model: Some(activity_model),
            allowed_tools,
            max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
            timeout_secs: request
                .timeout_secs
                .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS),
            subagent_type: Some(request.agent.key.clone()),
            run_in_background: false,
            cwd: Some(self.cwd_for_request(&request)),
            isolation: strict_workspace_boundary.then(|| "workspace-boundary".to_string()),
            provider_env,
        };

        let cancel = self
            .context
            .cancel_parent
            .as_ref()
            .map(|token| token.child_token())
            .unwrap_or_default();
        let mut tool_context = self.context.clone();
        tool_context.cancel_parent = Some(cancel.clone());

        let subagent_id = format!(
            "{}-{}-{}",
            request.session_id, request.ordinal, request.agent.key
        );
        let mut run: std::pin::Pin<Box<dyn std::future::Future<Output = SubagentOutcome> + Send>> =
            if request.disable_auto_background {
                Box::pin(run_subagent_foreground_with_system(
                    subagent_id,
                    req,
                    system,
                    cancel.clone(),
                    tool_context,
                ))
            } else {
                Box::pin(run_subagent_with_system(
                    subagent_id,
                    req,
                    system,
                    cancel.clone(),
                    tool_context,
                ))
            };
        let mut timed_out = false;
        let outcome = if let Some(timeout_secs) = request.timeout_secs {
            let timeout = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs.max(1)));
            tokio::pin!(timeout);
            tokio::select! {
                outcome = &mut run => outcome,
                _ = &mut timeout => {
                    timed_out = true;
                    cancel.cancel();
                    run.await
                }
            }
        } else {
            run.await
        };

        match outcome {
            SubagentOutcome::Completed(content) => Ok(LlmResponse {
                content,
                tool_uses: Vec::new(),
                tokens_in: 0,
                tokens_out: 0,
            }),
            SubagentOutcome::Failed(error) => Err(anyhow!("subagent failed: {error}")),
            SubagentOutcome::Cancelled if timed_out => Err(anyhow!(
                "subagent timed out after {}s",
                request
                    .timeout_secs
                    .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS)
            )),
            SubagentOutcome::Cancelled => Err(anyhow!("subagent cancelled")),
            SubagentOutcome::AutoBackgrounded => Err(anyhow!(
                "subagent auto-backgrounded before returning output"
            )),
        }
    }
}

fn workflow_provider_env_source(request: &AgentExecutionRequest) -> Option<ProviderEnvSource> {
    match (
        workflow_provider_env_policy(request),
        request.provider_env_resolution.clone(),
    ) {
        (Some(policy), Some(resolution)) => {
            Some(ProviderEnvSource::ResolvedPolicy { policy, resolution })
        }
        (Some(policy), None) => Some(ProviderEnvSource::Policy(policy)),
        (None, Some(resolution)) => Some(ProviderEnvSource::Resolution(resolution)),
        (None, None) => None,
    }
}

fn workflow_provider_env_policy(request: &AgentExecutionRequest) -> Option<ProviderEnvPolicy> {
    if request.pipeline_type != PipelineType::Workflow || !request.disable_auto_background {
        return None;
    }
    request
        .tools
        .iter()
        .find_map(provider_env_policy_from_marker)
}

fn values_to_text(values: &[serde_json::Value]) -> String {
    values
        .iter()
        .map(value_to_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn value_to_text(value: &serde_json::Value) -> String {
    if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
        return text.to_string();
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return text.to_string();
        }
        if let Some(parts) = content.as_array() {
            return values_to_text(parts);
        }
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
#[path = "subagent_adapter_tests.rs"]
mod tests;
