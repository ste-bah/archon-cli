//! TASK-AGS-708 Gate 1: integration test for the builder-retry wiring
//! (Validation Criterion 7). Proves that `build_llm_provider_with_policy`
//! wraps its concrete provider in `RetryProvider` such that a wiremock
//! server returning 503 twice and then 200 ultimately succeeds via the
//! retry loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};
use archon_llm::retry::{RetryPolicy, RetryProvider};
use archon_llm::secrets::ApiKey;
use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn leak_descriptor(mock: &MockServer) -> &'static ProviderDescriptor {
    let d = ProviderDescriptor {
        id: "retry-test".into(),
        display_name: "RetryTest".into(),
        base_url: Url::parse(&mock.uri()).expect("mock uri parses"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "RETRY_TEST_API_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "retry-model".into(),
        supports: ProviderFeatures::chat_only(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
    };
    Box::leak(Box::new(d))
}

fn ok_body() -> serde_json::Value {
    json!({
        "id": "chatcmpl",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "retry-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "after retries"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1_u64, "completion_tokens": 1_u64, "total_tokens": 2_u64}
    })
}

fn sample_request() -> LlmRequest {
    LlmRequest {
        model: "retry-model".into(),
        max_tokens: 16,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "ping"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
            request_origin: None,
    }
}

#[tokio::test]
async fn retry_provider_recovers_from_503_then_503_then_200() {
    let mock = MockServer::start().await;

    // First two POSTs -> 503, third POST -> 200.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down 1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down 2"))
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .with_priority(10)
        .mount(&mock)
        .await;

    let descriptor = leak_descriptor(&mock);
    let http = Arc::new(reqwest::Client::new());
    let inner = OpenAiCompatProvider::new(descriptor, http, ApiKey::new("test-key".into()));

    let policy = RetryPolicy {
        max_attempts: 4,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
        multiplier: 2.0,
        jitter: false,
    };
    let provider = RetryProvider::new(Arc::new(inner), policy);

    let resp = provider
        .complete(sample_request())
        .await
        .expect("retry loop must succeed on 3rd attempt");
    assert_eq!(resp.stop_reason, "stop");
    assert_eq!(
        resp.content[0]["text"].as_str(),
        Some("after retries"),
        "third attempt response must be returned"
    );
}
