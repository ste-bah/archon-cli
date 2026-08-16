use std::sync::Arc;

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_tools::agent_tool::SubagentRequest;

struct MockLlmProvider;

impl MockLlmProvider {
    fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_subagent_tools_does_not_panic_from_async_context() {
    let project_dir = std::env::temp_dir();
    let parent_permission_mode = Arc::new(tokio::sync::Mutex::new("default".to_string()));
    let pending_resume_messages =
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    let executor = AgentSubagentExecutor::new(
        Arc::new(MockLlmProvider::new()),
        ToolRegistry::new(),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(&project_dir))),
        None,
        None,
        project_dir.clone(),
        "test-session".into(),
        "claude-sonnet-4-6".into(),
        vec![],
        parent_permission_mode,
        pending_resume_messages,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".into(),
            String::new(),
            String::new(),
        )),
    );

    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    };

    // This must not panic with "Cannot block ... is being used to drive
    // asynchronous tasks" — was the v0.1.12 escape via blocking_lock at
    // subagent_executor.rs:210.
    let _ = executor.build_subagent_tools(&request, None).await;
}

/// An agent that cannot be spoken to is not a teammate (#184).
///
/// Found live: M1 fixed the routing so a subagent's `SendMessage` reaches its
/// target, and M5 made team members addressable by role — but no built-in agent
/// definition names `SendMessage`, so the `explore` agent asked to message a
/// teammate correctly reported it had no such tool. Working machinery nothing
/// can reach is the #153 shape.
///
/// The allowlist here is the narrowest a caller can express: one tool, named
/// explicitly. `SendMessage` must survive it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_subagent_can_always_reach_send_message() {
    let project_dir = std::env::temp_dir();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::send_message::SendMessageTool));

    let executor = AgentSubagentExecutor::new(
        Arc::new(MockLlmProvider::new()),
        registry,
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(&project_dir))),
        None,
        None,
        project_dir.clone(),
        "test-session".into(),
        "claude-sonnet-4-6".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".into(),
            String::new(),
            String::new(),
        )),
    );

    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec!["Read".into()],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: Some("explore".into()),
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    };

    let (defs, _filtered) = executor.build_subagent_tools(&request, None).await;
    let names: Vec<String> = defs
        .iter()
        .filter_map(|d| d.get("name")?.as_str().map(String::from))
        .collect();

    assert!(
        names.iter().any(|n| n == "SendMessage"),
        "a subagent must be able to answer its lead and its teammates, whatever \
         its allowlist says; got {names:?}"
    );
}
