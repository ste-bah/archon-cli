use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityConfigView, IdentityProvider, resolve_identity_mode};
use archon_llm::types::Secret;
use wiremock::matchers::{body_string_contains, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn bearer_oauth_token_sends_claude_code_identity_on_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer sk-ant-oat-wire-test"))
        .and(header("x-app", "cli"))
        .and(body_string_contains("You are Claude Code"))
        .and(body_string_contains("x-anthropic-billing-header"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let auth = AuthProvider::BearerToken(Secret::new("sk-ant-oat-wire-test".to_string()));
    let identity = IdentityProvider::new(
        resolve_identity_mode(&auth, false, &IdentityConfigView::default()),
        "session-wire".to_string(),
        "device-wire".to_string(),
        "account-wire".to_string(),
    );
    let client = AnthropicClient::new(auth, identity, Some(server.uri()));

    let request = MessageRequest {
        model: "claude-sonnet-4-6".to_string(),
        max_tokens: 16,
        messages: vec![serde_json::json!({
            "role": "user",
            "content": "wire test"
        })],
        ..MessageRequest::default()
    };

    client
        .stream_message(request)
        .await
        .expect("wiremock should accept spoofed OAuth request");

    let received = server
        .received_requests()
        .await
        .expect("wiremock should record the request");
    let request = received.last().expect("one request should be recorded");
    let betas = request
        .headers
        .get("anthropic-beta")
        .and_then(|value| value.to_str().ok())
        .expect("spoofed request must send anthropic-beta");
    assert!(
        betas.contains("oauth-2025-04-20"),
        "OAuth spoof request must include OAuth beta, got {betas}"
    );
}
