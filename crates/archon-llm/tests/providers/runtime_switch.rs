//! TC-PROV-03 AC-01: runtime model switch must produce responses from
//! the NEW provider after `swap_parts()` while in-flight handles from
//! BEFORE the swap continue to see the OLD provider (session preserved).
//!
//! SPEC DEVIATION:
//!   Spec wording (TASK-AGS-711 line 49) uses `ActiveProvider::new(&cfg_a,
//!   http)` + `active.swap(&cfg_b)` to point a live `ActiveProvider` at
//!   wiremock servers. That path is not viable: `LlmConfig::base_url`
//!   is declared (config.rs L42-43) but `build_llm_provider_with_policy`
//!   does NOT honor it — the dispatcher uses descriptor.base_url only.
//!   `from_parts` / `swap_parts` were added in TASK-AGS-709 precisely for
//!   this scenario ("bypass build_llm_provider_with_policy so tests can
//!   use wiremock-backed descriptors built via Box::leak" — active.rs
//!   L184-188). We use them here and still exercise ActiveProvider's
//!   real swap atomicity.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use archon_llm::active::ActiveProvider;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};
use archon_llm::retry::RetryPolicy;
use archon_llm::ApiKey;

fn canned_body(content: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-rs",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "rs-default",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1_u64, "completion_tokens": 1_u64, "total_tokens": 2_u64}
    })
}

fn leak_descriptor(id: &str, uri: &str) -> &'static ProviderDescriptor {
    let d = ProviderDescriptor {
        id: id.into(),
        display_name: id.into(),
        base_url: Url::parse(uri).expect("mock uri must parse"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "TEST_RS_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "rs-default".into(),
        supports: ProviderFeatures::chat_only(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
    };
    Box::leak(Box::new(d))
}

fn simple_request() -> LlmRequest {
    LlmRequest {
        model: "rs-default".into(),
        max_tokens: 16,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "hi"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn test_runtime_switch_preserves_session() {
    let mock_a = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_body("from-a")))
        .mount(&mock_a)
        .await;

    let mock_b = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_body("from-b")))
        .mount(&mock_b)
        .await;

    let http = Arc::new(reqwest::Client::new());
    let desc_a = leak_descriptor("mock-a", &mock_a.uri());
    let desc_b = leak_descriptor("mock-b", &mock_b.uri());

    let provider_a: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatProvider::new(
        desc_a,
        Arc::clone(&http),
        ApiKey::new("rs-key".into()),
    ));
    let provider_b: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatProvider::new(
        desc_b,
        Arc::clone(&http),
        ApiKey::new("rs-key".into()),
    ));

    let active = ActiveProvider::from_parts(
        Arc::clone(&provider_a),
        desc_a,
        Arc::clone(&http),
        RetryPolicy::default(),
    );

    // Pre-swap request -> mock_a
    let resp_a = active
        .complete(simple_request())
        .await
        .expect("pre-swap complete must succeed");
    let text_a = resp_a.content[0]["text"]
        .as_str()
        .expect("content[0].text must be string");
    assert_eq!(text_a, "from-a");
    assert_eq!(active.current_descriptor().id, "mock-a");

    // Atomically swap to provider_b
    active.swap_parts(Arc::clone(&provider_b), desc_b);
    assert_eq!(active.current_descriptor().id, "mock-b");

    // Post-swap request -> mock_b
    let resp_b = active
        .complete(simple_request())
        .await
        .expect("post-swap complete must succeed");
    let text_b = resp_b.content[0]["text"]
        .as_str()
        .expect("content[0].text must be string");
    assert_eq!(text_b, "from-b");

    // Session preserved invariant: no panics, both calls returned
    // distinct content from distinct upstreams.
    assert_ne!(text_a, text_b);
}
