//! TC-PROV-02: every entry in OPENAI_COMPAT_REGISTRY must successfully
//! roundtrip a chat completion against a wiremock server when its
//! base_url is overridden to point at the mock. Validates TASK-AGS-711
//! Validation Criteria 1, 2.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use archon_llm::ApiKey;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, list_compat,
};

fn canned_response() -> serde_json::Value {
    json!({
        "id": "chatcmpl-acceptance",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "acceptance-default",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ack"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1_u64, "completion_tokens": 1_u64, "total_tokens": 2_u64}
    })
}

fn simple_request() -> LlmRequest {
    LlmRequest {
        model: "acceptance-default".into(),
        max_tokens: 32,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "ping"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        request_origin: None,
        extra: serde_json::Value::Null,
    }
}

/// Clone a descriptor, override base_url to point at the mock, force
/// BearerApiKey auth flavor so auth header verification is uniform,
/// then Box::leak into a &'static that OpenAiCompatProvider::new can
/// accept. cargo restarts the process per test run, so the leak is
/// bounded per-run.
fn leak_with_mock(
    original: &'static ProviderDescriptor,
    mock_uri: &str,
) -> &'static ProviderDescriptor {
    let cloned = ProviderDescriptor {
        id: original.id.clone(),
        display_name: original.display_name.clone(),
        base_url: Url::parse(mock_uri).expect("mock uri must parse"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "TEST_ACCEPTANCE_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: original.default_model.clone(),
        supports: original.supports,
        headers: HashMap::new(), // drop static headers so matcher stays simple
        quirks: original.quirks,
    };
    Box::leak(Box::new(cloned))
}

#[tokio::test]
async fn test_all_compat_providers_roundtrip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer acceptance-test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response()))
        .mount(&server)
        .await;

    let http = Arc::new(reqwest::Client::new());
    let descriptors = list_compat();
    let expected = descriptors.len();
    assert_eq!(
        expected, 31,
        "OPENAI_COMPAT_REGISTRY must contain exactly 31 entries (TC-PROV-02 contract); got {expected}"
    );

    let mut successes = 0_usize;
    for original in descriptors {
        let patched = leak_with_mock(original, &server.uri());
        let provider = OpenAiCompatProvider::new(
            patched,
            Arc::clone(&http),
            ApiKey::new("acceptance-test-key".into()),
        );
        let resp = provider
            .complete(simple_request())
            .await
            .unwrap_or_else(|e| {
                panic!("provider {} must roundtrip; got error: {e:?}", original.id)
            });
        // Basic sanity on the canned body
        assert_eq!(
            resp.stop_reason, "stop",
            "provider {} stop_reason",
            original.id
        );
        successes += 1;
    }

    assert_eq!(
        successes, 31,
        "exactly 31 compat providers must roundtrip successfully"
    );

    let received = server
        .received_requests()
        .await
        .expect("mock must capture requests");
    assert_eq!(
        received.len(),
        31,
        "mock must receive exactly 31 POSTs (one per compat provider)"
    );
}
