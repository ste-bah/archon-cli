use std::path::Path;
use std::sync::Arc;

use archon_core::agent::AgentConfig;
use archon_llm::provider::LlmProvider;

pub(super) fn build_subagent_pipeline_client(
    provider: Arc<dyn LlmProvider>,
    agent_config: &AgentConfig,
    working_dir: &Path,
    session_id: &str,
) -> Arc<dyn archon_pipeline::runner::LlmClient> {
    let tool_context = archon_tools::tool::ToolContext {
        working_dir: working_dir.to_path_buf(),
        session_id: session_id.to_string(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: Vec::new(),
        in_fork: false,
        nested: false,
        cancel_parent: agent_config.cancel_token.clone(),
        sandbox: agent_config.sandbox.clone(),
        activity_sink: agent_config.activity_sink.clone(),
    };
    let raw: Arc<dyn archon_pipeline::runner::LlmClient> = Arc::new(
        archon_pipeline::llm_adapter::ProviderLlmAdapter::new(Arc::clone(&provider))
            .with_origin("tui_pipeline"),
    );
    Arc::new(
        archon_pipeline::subagent_adapter::SubagentPipelineClient::with_provider(
            raw,
            tool_context,
            provider,
        ),
    )
}
