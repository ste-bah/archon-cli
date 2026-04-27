use std::sync::Arc;

use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
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
    let pending_resume_messages = Arc::new(tokio::sync::Mutex::new(None));

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
    };

    // This must not panic with "Cannot block ... is being used to drive
    // asynchronous tasks" — was the v0.1.12 escape via blocking_lock at
    // subagent_executor.rs:210.
    let _ = executor.build_subagent_tools(&request, None).await;
}
