/// Integration tests for TASK-CLI-401: LlmProvider trait + AnthropicProvider adapter.
///
/// Gate 1: Tests written FIRST, before any implementation exists.
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature, ProviderRegistry,
};
use archon_llm::streaming::StreamEvent;

// ---------------------------------------------------------------------------
// Mock provider — used for registry tests (does not hit the network)
// ---------------------------------------------------------------------------

struct MockProvider {
    name: String,
}

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "mock-model-1".into(),
            display_name: "Mock Model 1".into(),
            context_window: 200_000,
        }]
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
        Err(LlmError::Unsupported(
            "mock does not support complete".into(),
        ))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Test 1: LlmProvider is object-safe — compile-time check
// ---------------------------------------------------------------------------

fn _check_object_safe(_: Box<dyn LlmProvider>) {}

// ---------------------------------------------------------------------------
// Test 2: ProviderRegistry stores and retrieves a provider
// ---------------------------------------------------------------------------

#[test]
fn registry_stores_and_retrieves_provider() {
    let mut registry = ProviderRegistry::new();
    let provider: Box<dyn LlmProvider> = Box::new(MockProvider {
        name: "mock".into(),
    });
    registry.register(provider);

    let found = registry.get("mock");
    assert!(found.is_some(), "should find the registered provider");
    assert_eq!(found.unwrap().name(), "mock");
}

// ---------------------------------------------------------------------------
// Test 3: Unknown provider returns None from get
// ---------------------------------------------------------------------------

#[test]
fn registry_unknown_provider_returns_none() {
    let registry = ProviderRegistry::new();
    let found = registry.get("nonexistent");
    assert!(found.is_none(), "unknown provider should return None");
}

// ---------------------------------------------------------------------------
// Test 4: AnthropicProvider::supports_feature recognises key features
// ---------------------------------------------------------------------------

#[test]
fn anthropic_supports_thinking_and_tool_use() {
    use archon_llm::anthropic::AnthropicClient;
    use archon_llm::auth::AuthProvider;
    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::providers::anthropic::AnthropicProvider;
    use archon_llm::types::Secret;

    let auth = AuthProvider::ApiKey(Secret::new("test-key".into()));
    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    );
    let client = AnthropicClient::new(auth, identity, None);
    let provider = AnthropicProvider::new(client);

    assert!(
        provider.supports_feature(ProviderFeature::Thinking),
        "Anthropic should support Thinking"
    );
    assert!(
        provider.supports_feature(ProviderFeature::ToolUse),
        "Anthropic should support ToolUse"
    );
    assert!(
        provider.supports_feature(ProviderFeature::Streaming),
        "Anthropic should support Streaming"
    );
    assert!(
        provider.supports_feature(ProviderFeature::SystemPrompt),
        "Anthropic should support SystemPrompt"
    );
}

// ---------------------------------------------------------------------------
// Test 5: LlmRequest converts from MessageRequest (round-trip fields)
// ---------------------------------------------------------------------------

#[test]
fn llm_request_from_message_request_round_trip() {
    use archon_llm::anthropic::MessageRequest;

    let msg_req = MessageRequest {
        model: "claude-sonnet-4-6".into(),
        max_tokens: 4096,
        system: vec![serde_json::json!({"type": "text", "text": "system"})],
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
        tools: vec![],
        thinking: Some(serde_json::json!({"type": "enabled", "budget_tokens": 1024})),
        speed: Some("fast".into()),
        effort: Some("low".into()),
        request_origin: None,
    };

    let llm_req: LlmRequest = msg_req.into();

    assert_eq!(llm_req.model, "claude-sonnet-4-6");
    assert_eq!(llm_req.max_tokens, 4096);
    assert_eq!(llm_req.system.len(), 1);
    assert_eq!(llm_req.messages.len(), 1);
    assert!(llm_req.thinking.is_some());
    assert_eq!(llm_req.speed.as_deref(), Some("fast"));
    assert_eq!(llm_req.effort.as_deref(), Some("low"));
}

// ---------------------------------------------------------------------------
// Test 6: ProviderRegistry::active returns Err with clear message for unknown
// ---------------------------------------------------------------------------

#[test]
fn registry_active_unknown_gives_clear_message() {
    let registry = ProviderRegistry::new();
    let result = registry.active("openai");
    assert!(
        result.is_err(),
        "active() should return Err for unknown provider"
    );
    let err_msg = result.err().unwrap().to_string();
    assert!(
        err_msg.contains("openai") || err_msg.contains("provider"),
        "error message should mention the provider name or 'provider': {err_msg}"
    );
}
