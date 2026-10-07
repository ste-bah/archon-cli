use super::*;

/// Abstraction over the underlying LLM transport. Concrete implementations
/// live in `archon-llm`; the pipeline crate depends only on this trait.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Digest of the rendered nonsecret request envelope; unknown means no reuse.
    fn message_request_identity(
        &self,
        _request: &archon_llm::provider::LlmRequest,
    ) -> Option<String> {
        None
    }

    fn provider_id(&self) -> Option<String> {
        None
    }

    /// May an agent this client spawns read `path`?
    ///
    /// Answered by the path guard the agent's own `Read`/`Glob`/`Grep` would
    /// consult, against the tool context every spawned agent inherits, so a
    /// host can learn before dispatching what it would otherwise learn from
    /// hours of the agent's tool errors. `None` means the client runs no tool
    /// sandbox (a plain completion client) and there is nothing to probe;
    /// `Some(Err(text))` carries the exact refusal the agent would have seen.
    fn probe_agent_read(&self, _path: &std::path::Path) -> Option<std::result::Result<(), String>> {
        None
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        model.to_string()
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse>;

    async fn send_message_with_temperature(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> Result<LlmResponse> {
        anyhow::bail!("client does not support explicit sampling")
    }

    /// Stream-aware explicit sampling. Nonstream clients stay silent until
    /// completion; they cannot renew an idle window without observed activity.
    async fn send_message_with_progress(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
        _progress: archon_shell::progress::Progress,
    ) -> Result<LlmResponse> {
        self.send_message_with_temperature(messages, system, tools, model, temperature)
            .await
    }

    /// Continue a completed invocation rather than creating a fresh agent.
    ///
    /// Refused by default: a continuation runs exactly the invocation it
    /// continues or not at all (#241). Only a client that restores the
    /// completed invocation exactly overrides this.
    async fn continue_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        Err(archon_tools::subagent_session::ContinuationRefused::no_session_kept().into())
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        let model = request.agent.model.clone();
        self.send_message(request.messages, request.system, request.tools, &model)
            .await
    }
}
