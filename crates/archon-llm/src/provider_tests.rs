//! Tests for `provider.rs`, held in their own file to keep that module under
//! the 500-line ceiling.

use super::*;

#[test]
fn classifies_local_and_cloud_endpoints() {
    assert_eq!(
        classify_data_flow_endpoint("http://localhost:11434/v1"),
        DataFlowClassification::Local
    );
    assert_eq!(
        classify_data_flow_endpoint("http://192.168.1.10:8080/v1"),
        DataFlowClassification::Local
    );
    assert_eq!(
        classify_data_flow_endpoint("https://api.anthropic.com/v1"),
        DataFlowClassification::Cloud
    );
}

struct DefaultAliasProvider;

#[async_trait::async_trait]
impl LlmProvider for DefaultAliasProvider {
    fn name(&self) -> &str {
        "default-alias"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "provider-default".into(),
            display_name: "Provider Default".into(),
            context_window: 128_000,
        }]
    }

    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported("test".into()))
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
}

#[test]
fn request_model_resolution_falls_back_for_tier_aliases() {
    let provider = DefaultAliasProvider;
    let mut request = LlmRequest {
        model: "opus".into(),
        ..LlmRequest::default()
    };
    provider.resolve_request_model(&mut request);
    assert_eq!(request.model, "provider-default");
}

/// A provider that says nothing about caching must not be treated as capable —
/// that default is what keeps an unknown endpoint from being sent directives it
/// may reject on every request.
#[test]
fn the_default_provider_declares_no_caching() {
    let provider = DefaultAliasProvider;
    assert_eq!(
        provider.cache_strategy("claude-sonnet-4-6"),
        crate::cache_strategy::CacheStrategy::None
    );
}
