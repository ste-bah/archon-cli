//! TASK-AGS-709 Validation Criterion 4: concurrent-swap integration test.
//!
//! Spawns 10 concurrent `complete` calls against an `ActiveProvider`
//! and, mid-flight, calls `swap()` to point it at a second mock server.
//! Asserts all 10 requests succeed (no panics, no dropped work, no
//! "session state" loss per US-PROV-03 AC-01).
//!
//! Uses `ActiveProvider::from_parts` to skip env-var plumbing and wire
//! in wiremock-backed descriptors directly.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use archon_llm::active::ActiveProvider;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};
use archon_llm::retry::RetryPolicy;
use archon_llm::secrets::ApiKey;
use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn leak_descriptor(id: &str, mock: &MockServer) -> &'static ProviderDescriptor {
    let d = ProviderDescriptor {
        id: id.to_string(),
        display_name: format!("Mock-{id}"),
        base_url: Url::parse(&mock.uri()).expect("mock uri parses"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "MOCK_API_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "mock-model".into(),
        supports: ProviderFeatures::chat_only(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
        is_gap: false,
    };
    Box::leak(Box::new(d))
}

fn ok_body(label: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": label},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1_u64, "completion_tokens": 1_u64, "total_tokens": 2_u64}
    })
}

fn sample_request() -> LlmRequest {
    LlmRequest {
        model: "mock-model".into(),
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
async fn test_concurrent_swap_and_request() {
    let mock_a = MockServer::start().await;
    let mock_b = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ok_body("from-a"))
                .set_delay(Duration::from_millis(30)),
        )
        .mount(&mock_a)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ok_body("from-b"))
                .set_delay(Duration::from_millis(30)),
        )
        .mount(&mock_b)
        .await;

    let desc_a = leak_descriptor("mock-a", &mock_a);
    let desc_b = leak_descriptor("mock-b", &mock_b);

    let http = Arc::new(reqwest::Client::new());
    let provider_a: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatProvider::new(
        desc_a,
        http.clone(),
        ApiKey::new("test-key".into()),
    ));
    let provider_b: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatProvider::new(
        desc_b,
        http.clone(),
        ApiKey::new("test-key".into()),
    ));

    let active = Arc::new(ActiveProvider::from_parts(
        provider_a,
        desc_a,
        http.clone(),
        RetryPolicy::default(),
    ));

    // Spawn 10 in-flight complete() calls against the current provider.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let a = active.clone();
        handles.push(tokio::spawn(async move {
            a.current().complete(sample_request()).await
        }));
    }

    // Give a couple of the in-flight requests time to start; then hot-swap.
    tokio::time::sleep(Duration::from_millis(5)).await;

    // Hot-swap via from_parts. This is the direct equivalent of `swap()`
    // for tests that don't go through env vars.
    active.swap_parts(provider_b.clone(), desc_b);

    // Launch another 5 AFTER the swap — these must hit mock_b.
    let mut post_swap_handles = Vec::new();
    for _ in 0..5 {
        let a = active.clone();
        post_swap_handles.push(tokio::spawn(async move {
            a.current().complete(sample_request()).await
        }));
    }

    // All in-flight (pre-swap) calls must complete successfully.
    let mut pre_swap_successes = 0;
    for h in handles {
        let resp = h
            .await
            .expect("task joins")
            .expect("request completes without error");
        assert_eq!(resp.stop_reason, "stop");
        pre_swap_successes += 1;
    }
    assert_eq!(
        pre_swap_successes, 10,
        "all 10 pre-swap requests must complete successfully"
    );

    // Post-swap requests must also succeed AND hit provider_b.
    let mut post_swap_successes = 0;
    for h in post_swap_handles {
        let resp = h
            .await
            .expect("task joins")
            .expect("post-swap request completes");
        assert_eq!(
            resp.content[0]["text"].as_str(),
            Some("from-b"),
            "post-swap request must route to provider_b"
        );
        post_swap_successes += 1;
    }
    assert_eq!(post_swap_successes, 5);

    assert_eq!(
        active.current_descriptor().id,
        "mock-b",
        "descriptor must reflect the swap"
    );
}
