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
use archon_tools::agent_tool::{SubagentRequest, run_subagent};
use archon_tools::subagent_executor::SubagentOutcome;
use archon_tools::tool::ToolContext;

use crate::runner::{AgentExecutionRequest, LlmClient, LlmResponse, ToolAccessLevel};

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

    fn prompt_for_request(request: &AgentExecutionRequest) -> String {
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

        let system_text = values_to_text(&request.system);
        if !system_text.trim().is_empty() {
            parts.push(format!("## Runtime System Context\n{system_text}"));
        }

        let message_text = values_to_text(&request.messages);
        if !message_text.trim().is_empty() {
            parts.push(format!("## Agent Prompt\n{message_text}"));
        }

        parts.join("\n\n")
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
        let req = SubagentRequest {
            prompt,
            model: Some(activity_model),
            allowed_tools: Self::allowed_tools(&request),
            max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
            timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
            subagent_type: Some(request.agent.key.clone()),
            run_in_background: false,
            cwd: Some(self.cwd_for_request(&request)),
            isolation: None,
        };

        let cancel = self
            .context
            .cancel_parent
            .as_ref()
            .map(|token| token.child_token())
            .unwrap_or_default();

        let outcome = run_subagent(
            format!(
                "{}-{}-{}",
                request.session_id, request.ordinal, request.agent.key
            ),
            req,
            cancel,
            self.context.clone(),
        )
        .await;

        match outcome {
            SubagentOutcome::Completed(content) => Ok(LlmResponse {
                content,
                tool_uses: Vec::new(),
                tokens_in: 0,
                tokens_out: 0,
            }),
            SubagentOutcome::Failed(error) => Err(anyhow!("subagent failed: {error}")),
            SubagentOutcome::Cancelled => Err(anyhow!("subagent cancelled")),
            SubagentOutcome::AutoBackgrounded => Err(anyhow!(
                "subagent auto-backgrounded before returning output"
            )),
        }
    }
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
mod tests {
    use super::*;
    use crate::runner::{AgentInfo, PipelineType};
    use archon_llm::provider::{
        LlmError, LlmResponse as ProviderResponse, ModelInfo, ProviderFeature,
    };

    struct NoopClient;

    #[async_trait]
    impl LlmClient for NoopClient {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: String::new(),
                tool_uses: Vec::new(),
                tokens_in: 0,
                tokens_out: 0,
            })
        }
    }

    struct AliasProvider;

    #[async_trait]
    impl LlmProvider for AliasProvider {
        fn name(&self) -> &str {
            "openai-codex"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo {
                id: "gpt-5.4".into(),
                display_name: "GPT-5.4".into(),
                context_window: 1_050_000,
            }]
        }

        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError>
        {
            Err(LlmError::Unsupported("not used".into()))
        }

        async fn complete(&self, _request: LlmRequest) -> Result<ProviderResponse, LlmError> {
            Err(LlmError::Unsupported("not used".into()))
        }

        fn supports_feature(&self, _feature: ProviderFeature) -> bool {
            false
        }
    }

    fn request(access: ToolAccessLevel) -> AgentExecutionRequest {
        AgentExecutionRequest {
            session_id: "s".into(),
            pipeline_type: PipelineType::Coding,
            task: "task".into(),
            cwd: None,
            ordinal: 1,
            attempt: 1,
            agent: AgentInfo {
                key: "context-gatherer".into(),
                display_name: "Context Gatherer".into(),
                model: "sonnet".into(),
                phase: 1,
                critical: false,
                parallelizable: false,
                quality_threshold: 0.5,
                tool_access_level: access,
            },
            messages: vec![serde_json::json!({"role":"user","content":"hello"})],
            system: vec![serde_json::json!({"type":"text","text":"system"})],
            tools: Vec::new(),
            allowed_tools: Vec::new(),
        }
    }

    #[test]
    fn read_only_tools_include_memory_and_docs_but_not_writes() {
        let tools = SubagentPipelineClient::allowed_tools(&request(ToolAccessLevel::ReadOnly));
        assert!(tools.contains(&"memory_recall".to_string()));
        assert!(tools.contains(&"DocSearch".to_string()));
        assert!(!tools.contains(&"Write".to_string()));
        assert!(!tools.contains(&"Bash".to_string()));
    }

    #[test]
    fn full_tools_include_write_and_memory_store() {
        let tools = SubagentPipelineClient::allowed_tools(&request(ToolAccessLevel::Full));
        assert!(tools.contains(&"Write".to_string()));
        assert!(tools.contains(&"memory_store".to_string()));
        assert!(tools.contains(&"ApplyPatch".to_string()));
    }

    #[test]
    fn activity_model_resolves_tier_alias_with_active_provider() {
        let client = SubagentPipelineClient::with_provider(
            Arc::new(NoopClient),
            ToolContext::default(),
            Arc::new(AliasProvider),
        );

        assert_eq!(client.activity_model("sonnet"), "gpt-5.4");
    }

    #[test]
    fn request_cwd_overrides_parent_context() {
        let client = SubagentPipelineClient::new(
            Arc::new(NoopClient),
            ToolContext {
                working_dir: "/project/root".into(),
                ..ToolContext::default()
            },
        );
        let mut request = request(ToolAccessLevel::Full);
        request.cwd = Some("/target/repo".into());

        assert_eq!(client.cwd_for_request(&request), "/target/repo");
    }
}
