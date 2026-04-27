//! Asserts that in spoof mode, every outgoing API request carries
//! `x-anthropic-billing-header` with a valid `cch=<5hex>` token computed
//! from the request body.
//!
//! Also verifies that clean mode omits the header.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::AuthProvider;
use archon_llm::cch::compute_cch;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::types::Secret;

fn spoof_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec!["claude-code-20250219".into(), "oauth-2025-04-20".into()],
            workload: None,
            anti_distillation: false,
        },
        "test-session".into(),
        "test-device".into(),
        String::new(),
    )
}

fn clean_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "test-session".into(),
        "test-device".into(),
        String::new(),
    )
}

fn auth() -> AuthProvider {
    AuthProvider::ApiKey(Secret::new("test-api-key".to_string()))
}

fn test_request() -> MessageRequest {
    MessageRequest {
        model: "claude-sonnet-4-6".into(),
        max_tokens: 64,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        request_origin: Some("test".into()),
    }
}

#[tokio::test]
async fn spoof_mode_includes_x_anthropic_billing_header() {
    let server = MockServer::start().await;

    // Stub: return a minimal SSE body so stream_message doesn't error
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::new(auth(), spoof_identity(), Some(api_url));

    // stream_message will error on empty SSE stream (no message_start), but the
    // HTTP request is sent and captured by wiremock before that happens.
    let _ = client.stream_message(test_request()).await;

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1, "one request should have been sent");

    let billing = requests[0]
        .headers
        .get("x-anthropic-billing-header")
        .expect("spoof mode must include x-anthropic-billing-header")
        .to_str()
        .unwrap()
        .to_string();

    // Verify format: cc_version=...; cc_entrypoint=...; cch=XXXXX; cc_workload=...;
    assert!(
        billing.starts_with("cc_version="),
        "billing header must start with cc_version=: {billing}"
    );
    assert!(
        billing.contains("cc_entrypoint=claude_code"),
        "billing header must contain cc_entrypoint=claude_code: {billing}"
    );
    assert!(
        billing.contains("cc_workload=claude_code"),
        "billing header must contain cc_workload=claude_code: {billing}"
    );

    // Verify cch=<5hex> is present and valid
    let cch_pos = billing
        .find("cch=")
        .expect("billing header must contain cch=");
    let after_cch = &billing[cch_pos..];
    let cch_part: String = after_cch
        .chars()
        .skip(4) // skip "cch="
        .take(5)
        .collect();
    assert_eq!(
        cch_part.len(),
        5,
        "cch must be 5 hex digits, got: '{cch_part}'"
    );
    assert!(
        cch_part.chars().all(|c: char| c.is_ascii_hexdigit()),
        "cch must be lowercase hex, got: '{cch_part}'"
    );

    // Verify cch matches the actual request body
    let body = &requests[0].body;
    let expected_cch = compute_cch(body);
    assert!(
        billing.contains(&expected_cch),
        "billing header cch must match compute_cch(body). Expected: {expected_cch}, got header: {billing}"
    );
}

#[tokio::test]
async fn clean_mode_omits_x_anthropic_billing_header() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let api_url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::new(auth(), clean_identity(), Some(api_url));

    let _ = client.stream_message(test_request()).await;

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);

    assert!(
        requests[0]
            .headers
            .get("x-anthropic-billing-header")
            .is_none(),
        "clean mode must NOT include x-anthropic-billing-header"
    );
}
