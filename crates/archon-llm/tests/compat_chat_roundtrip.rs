//! TASK-AGS-703 Gate 1: integration tests for `OpenAiCompatProvider`.
//! Written BEFORE implementation — these pin the public behavior so
//! the implementation has a compile-and-pass target.
//!
//! TASK-AGS-703 SPEC DEVIATION (greenlit 2026-04-13):
//!   ChatRequest    -> LlmRequest  (existing provider-agnostic type)
//!   ChatResponse   -> LlmResponse (existing provider-agnostic type)
//!   .chat()        -> .complete() (existing trait method)
//!   .stream_chat() -> .stream()   (existing trait method)
//!   ProviderError  -> mapped to LlmError at the trait boundary
//!   .embed()       -> inherent method on OpenAiCompatProvider, not trait
//!
//! Spec validation criteria (TASK-AGS-703 §Validation Criteria):
//!   (2) happy-path chat roundtrip → `chat_roundtrip_happy_path`
//!   (3) Bearer auth header sent → `bearer_auth_header_sent`
//!   (4) no auth header when `AuthFlavor::None` → `no_auth_header_for_none_flavor`
//!   (5) 401 → `LlmError::Auth` → `http_401_maps_to_auth_error`
//!   (6) 503 → `LlmError::Server`/`Http` → `http_503_maps_to_server_error`

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use url::Url;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use archon_llm::ApiKey;
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a `&'static ProviderDescriptor` pointed at the given mock server
/// URI. `Box::leak` is acceptable in test scope; cargo re-starts the
/// process per test invocation so the leak is bounded.
fn make_descriptor(uri: &str, auth_flavor: AuthFlavor) -> &'static ProviderDescriptor {
    let env_key_var = match &auth_flavor {
        AuthFlavor::BearerApiKey => "TEST_API_KEY".to_string(),
        _ => String::new(),
    };
    let d = ProviderDescriptor {
        id: "test_compat".into(),
        display_name: "TestCompat".into(),
        base_url: Url::parse(uri).expect("wiremock uri must parse"),
        auth_flavor,
        env_key_var,
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "test-default-model".into(),
        supports: ProviderFeatures::chat_only(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
    };
    Box::leak(Box::new(d))
}

fn make_provider(descriptor: &'static ProviderDescriptor, key: &str) -> OpenAiCompatProvider {
    OpenAiCompatProvider::new(
        descriptor,
        Arc::new(reqwest::Client::new()),
        ApiKey::new(key.to_string()),
    )
}

fn canned_chat_completion_body(content: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "test-default-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5_u64, "completion_tokens": 10_u64, "total_tokens": 15_u64}
    })
}

fn simple_user_request(model: &str) -> LlmRequest {
    LlmRequest {
        model: model.to_string(),
        max_tokens: 256,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "hi there"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
            request_origin: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chat_roundtrip_happy_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(canned_chat_completion_body("hello from mock")),
        )
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "test-key");

    let resp = provider
        .complete(simple_user_request("test-default-model"))
        .await
        .expect("happy path must succeed");

    assert!(
        !resp.content.is_empty(),
        "response content must be non-empty"
    );
    let text = resp.content[0]["text"]
        .as_str()
        .expect("content[0].text must be a string");
    assert_eq!(text, "hello from mock");
    assert_eq!(resp.stop_reason, "stop");
    assert_eq!(resp.usage.input_tokens, 5);
    assert_eq!(resp.usage.output_tokens, 10);
}

#[tokio::test]
async fn bearer_auth_header_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_chat_completion_body("ok")))
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "test-key");

    let resp = provider
        .complete(simple_user_request("test-default-model"))
        .await;
    assert!(
        resp.is_ok(),
        "request without bearer header would fail the matcher; got: {resp:?}"
    );
}

#[tokio::test]
async fn no_auth_header_for_none_flavor() {
    let server = MockServer::start().await;
    // A matcher that REQUIRES the absence of a header isn't a first-class
    // wiremock primitive, so use a two-mount strategy: a high-priority
    // rejection mock that fires if Authorization is present, and a
    // default-priority success mock otherwise.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(499).set_body_string("should not have sent auth"))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_chat_completion_body("ok")))
        .with_priority(10)
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::None);
    // Key is ignored for AuthFlavor::None, but still supplied by constructor.
    let provider = make_provider(descriptor, "unused");

    let resp = provider
        .complete(simple_user_request("test-default-model"))
        .await;
    assert!(
        resp.is_ok(),
        "AuthFlavor::None must NOT send an Authorization header; got: {resp:?}"
    );
}

#[tokio::test]
async fn http_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "bad key"}})),
        )
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "wrong-key");

    let err = provider
        .complete(simple_user_request("test-default-model"))
        .await
        .expect_err("401 must map to Err");
    match err {
        LlmError::Auth(_) => {}
        other => panic!("expected LlmError::Auth, got {other:?}"),
    }
}

#[tokio::test]
async fn http_503_maps_to_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "any");

    let err = provider
        .complete(simple_user_request("test-default-model"))
        .await
        .expect_err("503 must map to Err");
    match err {
        // Mapping: ProviderError::Unreachable → LlmError::Server or LlmError::Http
        LlmError::Server { status: 503, .. } => {}
        LlmError::Http(_) => {}
        other => panic!("expected LlmError::Server{{503}} or Http, got {other:?}"),
    }
}

#[tokio::test]
async fn empty_model_uses_descriptor_default() {
    let server = MockServer::start().await;
    // Match the body contains the default model id.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_chat_completion_body("ok")))
        .mount(&server)
        .await;

    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "k");

    // Empty model in the request → provider should substitute descriptor.default_model.
    let resp = provider
        .complete(simple_user_request(""))
        .await
        .expect("empty model request must succeed via default substitution");
    assert_eq!(resp.stop_reason, "stop");

    // Capture the request body via the recorded matches.
    let recv = server.received_requests().await.expect("requests recorded");
    let last = recv.last().expect("at least one request");
    let body: serde_json::Value =
        serde_json::from_slice(&last.body).expect("request body must parse as JSON");
    assert_eq!(
        body["model"].as_str(),
        Some("test-default-model"),
        "empty model must be substituted with descriptor.default_model"
    );
}

#[tokio::test]
async fn name_returns_display_name() {
    let server = MockServer::start().await;
    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "k");
    assert_eq!(provider.name(), "TestCompat");
}

#[tokio::test]
async fn models_exposes_descriptor_default_model() {
    let server = MockServer::start().await;
    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "k");
    let models = provider.models();
    assert!(
        models.iter().any(|m| m.id == "test-default-model"),
        "models() must surface descriptor.default_model; got: {models:?}"
    );
}

#[tokio::test]
async fn stream_performs_real_network_call_post_task_707() {
    // Post-TASK-AGS-707: the stream() placeholder (`LlmError::Unsupported("…deferred…")`)
    // is gone. `stream()` now issues a real POST; on a wiremock server with
    // no mount the request surfaces as `LlmError::Http` carrying a 404
    // status. That proves the method is wired end-to-end and no longer
    // short-circuits with the TASK-AGS-707 placeholder. Positive-path
    // streaming (3 frames + [DONE], NDJSON, unsupported short-circuit,
    // 401 mapping, truncated-stream close) is covered in
    // `tests/compat_stream_sse.rs` and `tests/compat_stream_ndjson.rs`.
    let server = MockServer::start().await;
    let descriptor = make_descriptor(&server.uri(), AuthFlavor::BearerApiKey);
    let provider = make_provider(descriptor, "k");
    let err = provider
        .stream(simple_user_request("test-default-model"))
        .await
        .expect_err("stream() against unmounted wiremock must Err on the 404");
    let s = format!("{err}");
    let lower = s.to_lowercase();
    assert!(
        !lower.contains("deferred") && !s.contains("707"),
        "post-707 error must NOT reference the TASK-AGS-707 placeholder; got: {s}"
    );
    assert!(
        lower.contains("404") || lower.contains("http"),
        "expected an LlmError::Http referencing 404; got: {s}"
    );
}
