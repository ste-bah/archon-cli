use archon_llm::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, ProviderRegistry,
};
use archon_workflow::{ProviderTier, ProviderTierResolver};

struct MockProvider {
    name: &'static str,
    model: &'static str,
    vision: bool,
    flow: DataFlowClassification,
}

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.model.into(),
            display_name: "Mock".into(),
            context_window: 128_000,
        }]
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported("mock".into()))
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        feature == ProviderFeature::Vision && self.vision
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        self.flow
    }
}

fn registry(provider: MockProvider) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(provider));
    registry
}

#[test]
fn resolves_through_neutral_provider_registry() {
    let registry = registry(MockProvider {
        name: "openai-codex",
        model: "gpt-test",
        vision: true,
        flow: DataFlowClassification::Cloud,
    });
    let resolved = ProviderTierResolver::new("openai-codex", "ignored")
        .resolve_with_registry(ProviderTier::Planner, &registry)
        .unwrap();
    assert_eq!(resolved.provider_id, "openai-codex");
    assert_eq!(resolved.model, "gpt-test");
    assert_eq!(resolved.data_flow, "cloud");
    assert!(resolved.features.contains(&"vision".to_string()));
}

#[test]
fn vision_tier_requires_provider_vision_feature() {
    let registry = registry(MockProvider {
        name: "openai-codex",
        model: "gpt-test",
        vision: false,
        flow: DataFlowClassification::Cloud,
    });
    let err = ProviderTierResolver::new("openai-codex", "ignored")
        .resolve_with_registry(ProviderTier::Vision, &registry)
        .unwrap_err();
    assert!(err.to_string().contains("vision tier requires"));
}

#[test]
fn local_tier_requires_local_data_flow() {
    let registry = registry(MockProvider {
        name: "openai-codex",
        model: "gpt-test",
        vision: true,
        flow: DataFlowClassification::Cloud,
    });
    let err = ProviderTierResolver::new("openai-codex", "ignored")
        .resolve_with_registry(ProviderTier::Local, &registry)
        .unwrap_err();
    assert!(err.to_string().contains("local tier requires"));
}
