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
use archon_tools::workflow_read_guard::{TreeWideMutator, WorkflowReadGuardSettings};

use crate::runner::{AgentExecutionRequest, LlmClient, LlmResponse, PipelineType, ToolAccessLevel};

mod continuation;

const EXACT_TOOL_POLICY_MARKER: &str = "__ARCHON_EXACT_TOOLS__";

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

/// Whether `name` is in the read-only vocabulary a `ReadOnly` agent is
/// offered by default.
///
/// Exposed for the workflow host's native-tool admission (Issue-28): a task
/// may declare a native tool by name, and a read-only stage may be given it
/// only if this list already contains it — the same list that bounds the
/// stage when nothing is declared, so a declaration cannot widen it.
pub fn is_read_only_tool(name: &str) -> bool {
    READ_ONLY_TOOLS.contains(&name)
}

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
    /// `[workflow] write_confinement`. The single switch, read in exactly one
    /// place — [`Self::declared_write_roots`].
    write_confinement: bool,
    workflow_read_guard: WorkflowReadGuardSettings,
    sessions: continuation::SessionCache,
}

impl SubagentPipelineClient {
    pub fn new(fallback: Arc<dyn LlmClient>, context: ToolContext) -> Self {
        Self {
            fallback,
            context,
            activity_provider: None,
            write_confinement: false,
            workflow_read_guard: WorkflowReadGuardSettings::default(),
            sessions: Default::default(),
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
            write_confinement: false,
            workflow_read_guard: WorkflowReadGuardSettings::default(),
            sessions: Default::default(),
        }
    }

    /// Enforce `[workflow] write_confinement` for the agents this client spawns.
    ///
    /// A builder rather than a constructor argument so the two existing
    /// constructors keep their current meaning: every caller that does not say
    /// this is unconfined, exactly as before.
    #[must_use]
    pub fn with_write_confinement(mut self, enabled: bool) -> Self {
        self.write_confinement = enabled;
        self
    }

    #[must_use]
    pub fn with_workflow_read_guard(mut self, max_reads: u32, reads_per_write: u32, allow_release_builds: bool, allow_git_mutation: bool) -> Self {
        self.workflow_read_guard.max_reads_before_first_write = max_reads;
        self.workflow_read_guard.reads_per_write = reads_per_write;
        self.workflow_read_guard.allow_release_builds = allow_release_builds;
        self.workflow_read_guard.allow_git_mutation = allow_git_mutation;
        self
    }

    /// `[workflow.generated] tree_wide_mutators` / `allow_tree_wide_mutators`:
    /// the formatter and fixer shapes the guard refuses unless scoped, and the
    /// operator switch that lets them run over the whole tree.
    #[must_use]
    pub fn with_tree_wide_mutators(mut self, rules: Vec<TreeWideMutator>, allow: bool) -> Self {
        self.workflow_read_guard.tree_wide_mutators = rules;
        self.workflow_read_guard.allow_tree_wide_mutators = allow;
        self
    }

    /// The directories this agent may write, or empty for unconfined.
    ///
    /// This is the whole scope of the feature, and the three guards are all
    /// here so there is one place to read rather than three to reconcile.
    ///
    /// **Workflow only.** `PipelineType` is checked rather than assumed:
    /// `SubagentPipelineClient` also backs interactive pipeline stages, and
    /// confining those would silently demote a directory the user added with
    /// `/add-dir` — which they added because they intend to edit in it — to
    /// read-only. The `Agent` and `TaskCreate` tools build their own
    /// `SubagentRequest` and never come through here at all, so an interactive
    /// subagent cannot reach this code by any route.
    ///
    /// **Declared or nothing.** An enabled knob over an undeclared run confines
    /// nothing, and warns. The tempting alternative is to fall back to the
    /// agent's working directory; that is what an earlier attempt did, and it
    /// refuses the deliverable of every workflow whose artifacts live outside
    /// the tree the agent runs in — which is the normal shape, not the exotic
    /// one.
    ///
    /// **The workspace is added, not substituted.** An agent must be able to
    /// write where it was told to work, whether or not the declaration happened
    /// to name it. The addition is host-derived — `cwd_for_request` reads the
    /// call's own working directory — so it cannot be steered by the agent.
    fn declared_write_roots(&self, request: &AgentExecutionRequest) -> Vec<String> {
        if !self.write_confinement || request.pipeline_type != PipelineType::Workflow {
            return Vec::new();
        }
        if request.write_roots.is_empty() {
            tracing::warn!(
                session_id = %request.session_id,
                agent = %request.agent.key,
                "workflow.write_confinement is enabled but this run declared no artifact or \
                 repository roots; writes stay unconfined for this agent"
            );
            return Vec::new();
        }
        let mut roots = request.write_roots.clone();
        let workspace = self.cwd_for_request(request);
        if !roots.iter().any(|root| root == &workspace) {
            roots.push(workspace);
        }
        roots
    }

    fn allowed_tools(request: &AgentExecutionRequest) -> Vec<String> {
        let mut tools = if !request.allowed_tools.is_empty() { request.allowed_tools.clone() } else {
            let source = match request.agent.tool_access_level {
                ToolAccessLevel::ReadOnly => READ_ONLY_TOOLS,
                ToolAccessLevel::Full => FULL_TOOLS,
            };
            source.iter().map(|tool| (*tool).to_string()).collect()
        };
        if let Some(landing) = archon_tools::audit_landing::current() {
            if !tools.iter().any(|t|t==landing.tool_name()) { tools.push(landing.tool_name().into()); }
        }
        tools
    }

    fn prompt_for_request(request: &AgentExecutionRequest) -> SubagentPipelinePrompt {
        let message_text = values_to_text(&request.messages);
        let task_in_message = !request.task.is_empty()
            && (message_text == request.task || message_text.contains(&format!(
                "## Task\n{}\n\n## Input\n", request.task
            )));
        let task_section = if task_in_message { String::new() }
            else { format!("\n\n## Pipeline Task\n{}", request.task) };
        let mut parts = vec![format!(
            "## Pipeline Agent Run\nPipeline: {:?}\nSession: {}\nAgent: {} ({})\nPhase: {}\nOrdinal: {}\nAttempt: {}{}",
            request.pipeline_type,
            request.session_id,
            request.agent.key,
            request.agent.display_name,
            request.agent.phase,
            request.ordinal,
            request.attempt,
            task_section
        )];

        parts.push(format!(
            "## Archon Tool Contract\nUse only these Archon tool names for this run: {}.\nAny `mcp__server__tool` name in that list is a PROJECT MCP tool configured for this repository: call it directly when the task asks for it. Any other name in that list that the task declared (its required_tools/tools/allowed_tools) is a native Archon tool: call it directly, and never substitute a shell command for a declared tool — the run is checked for the declared name, not for an equivalent. What is forbidden is the legacy Claude Flow, God pipeline and ruv-swarm vocabulary — do not call those names even if old imported agent text mentions them, and do not run `claude-flow` or `npx ruv-swarm` through Bash. Map code search to LeannSearch/lsp/Grep/Read, memory work to memory_recall/memory_store, research/doc work to Doc*/WebSearch/WebFetch, and delegation to Agent.",
            Self::allowed_tools(request)
                .into_iter()
                .filter(|tool| tool != EXACT_TOOL_POLICY_MARKER)
                .collect::<Vec<_>>()
                .join(", ")
        ));

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
            && (request.agent.tool_access_level == ToolAccessLevel::Full
                || allowed_tools.iter().any(|tool| tool == EXACT_TOOL_POLICY_MARKER))
            && request.cwd.is_some()
            && !allowed_tools
                .iter()
                .any(|tool| tool.eq_ignore_ascii_case("Bash"))
    }
}

#[async_trait]
impl LlmClient for SubagentPipelineClient {
    fn provider_id(&self) -> Option<String> {
        self.fallback.provider_id()
    }
    fn resolve_model_alias(&self, model: &str) -> String {
        self.fallback.resolve_model_alias(model)
    }

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

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> Result<LlmResponse> {
        self.fallback
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.execute_session(request, false).await
    }

    async fn continue_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.execute_session(request, true).await
    }

}

/// Map a terminal [`SubagentOutcome`] onto the pipeline's response type.
///
/// `Completed` is the runner's own typed statement that it finished normally —
/// every other ending has its own variant and becomes an error here — so it is
/// reported as `end_turn` rather than as an absent stop reason. Callers that
/// require a typed terminal reason (the fixed decomposition author) can then
/// tell a finished turn from one that never produced one, which a hardcoded
/// `None` made impossible for every subagent-backed provider.
pub(crate) fn llm_response_for_subagent_outcome(
    outcome: SubagentOutcome,
    timed_out: bool,
    timeout_secs: Option<u64>,
) -> Result<LlmResponse> {
    match outcome {
        SubagentOutcome::Completed(content) => Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
            stop_reason: Some("end_turn".to_string()),
        }),
        SubagentOutcome::Failed(error) => Err(anyhow!("subagent failed: {error}")),
        SubagentOutcome::Cancelled if timed_out => Err(anyhow!(
            "subagent timed out after {}s",
            timeout_secs.unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS)
        )),
        SubagentOutcome::Cancelled => Err(anyhow!("subagent cancelled")),
        SubagentOutcome::AutoBackgrounded => Err(anyhow!(
            "subagent auto-backgrounded before returning output"
        )),
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
