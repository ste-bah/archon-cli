use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::providers::anthropic::AnthropicProvider;
use archon_llm::types::Secret;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[tokio::test]
async fn workflow_system_prefix_reaches_anthropic_wire_body() {
    let server = MockServer::start().await;
    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;
    let provider = Arc::new(AnthropicProvider::new(AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        ),
        Some(format!("{}/v1/messages", server.uri())),
    )));
    let stable = serde_json::json!({
        "type":"text",
        "text":"stable workflow universe",
        "cache_control":{"type":"ephemeral"}
    });
    let mut runner = make_runner(provider, 1);
    runner.set_request_system(vec![stable.clone()]);

    runner.run("call_id: volatile-call").await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let system = body["system"].as_array().unwrap();
    assert!(system.iter().any(|block| {
        block.get("text").and_then(serde_json::Value::as_str) == Some("stable workflow universe")
    }));
    assert!(
        system
            .iter()
            .all(|block| block.get("cache_control").is_none())
    );
    assert!(body["messages"].to_string().contains("volatile-call"));
    assert!(
        !body["messages"]
            .to_string()
            .contains("stable workflow universe")
    );
}
