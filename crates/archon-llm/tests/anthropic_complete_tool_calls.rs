use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::anthropic::AnthropicProvider;
use archon_llm::types::Secret;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn anthropic_complete_preserves_tool_use_blocks() {
    let server = MockServer::start().await;
    let sse = [
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-4-6","usage":{"input_tokens":4,"output_tokens":0}}}"#,
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Need lookup."}}"#,
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#,
        r#"event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Lookup","input":{}}}"#,
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"arc"}}"#,
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"hon\"}"}}"#,
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":1}"#,
        r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":2}}"#,
        r#"event: message_stop
data: {"type":"message_stop"}"#,
    ]
    .join("\n\n");

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let auth = AuthProvider::ApiKey(Secret::new("test-key".into()));
    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    );
    let client = AnthropicClient::new(
        auth,
        identity,
        Some(format!("{}/v1/messages", server.uri())),
    );
    let provider = AnthropicProvider::new(client);

    let response = provider
        .complete(LlmRequest {
            model: "claude-sonnet-4-6".into(),
            messages: vec![serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "look this up"}]
            })],
            ..LlmRequest::default()
        })
        .await
        .expect("complete");

    assert_eq!(
        response.content[0],
        serde_json::json!({"type": "text", "text": "Need lookup."})
    );
    assert_eq!(
        response.content[1],
        serde_json::json!({
            "type": "tool_use",
            "id": "toolu_1",
            "name": "Lookup",
            "input": {"query": "archon"},
        })
    );
    assert_eq!(response.usage.input_tokens, 4);
    assert_eq!(response.usage.output_tokens, 2);
    assert_eq!(response.stop_reason, "tool_use");
}
