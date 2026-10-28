use archon_llm::{
    provider::{LlmProvider, LlmRequest},
    providers::{
        AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
        ProviderQuirks,
    },
    secrets::ApiKey,
};
use std::{sync::Arc, time::Duration};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn structured_permanent_payloads_survive_compatible_http_mapping() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let provider = OpenAiCompatProvider::new(Box::leak(Box::new(ProviderDescriptor {
            id: "AC-X-001".into(), display_name: "AC-X-001".into(),
            base_url: server.uri().parse().unwrap(), auth_flavor: AuthFlavor::None,
            env_key_var: String::new(), compat_kind: CompatKind::OpenAiCompat,
            default_model: "AC-X-001".into(), supports: ProviderFeatures::chat_only(),
            headers: Default::default(), quirks: ProviderQuirks::default(), is_gap: false,
        })), Arc::new(archon_llm::transport_http::client_builder().build().unwrap()),
        ApiKey::new("AC-X-001".into()));
        for (status, code) in [(429, "insufficient_quota"), (500, "model_not_found")] {
            server.reset().await;
            Mock::given(wiremock::matchers::method("POST")).respond_with(ResponseTemplate::new(status)
                .set_body_json(serde_json::json!({"error":{"code":code,"message":"set request.value=AC-X-001"}})))
                .mount(&server).await;
            let error = archon_llm::provider_fault::defer_retry(provider.stream(LlmRequest {
                model: "AC-X-001".into(), ..Default::default()
            })).await.unwrap_err();
            assert!(error.is_permanent(), "structured rejection became transient: {error:?}");
            assert!(error.to_string().contains("set request.value=AC-X-001"));
        }
    }).await.unwrap();
}
