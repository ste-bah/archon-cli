use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::codex::client::CodexProvider;
use archon_llm::providers::codex::spoof_default::SpoofConfig;
use archon_llm::streaming::StreamEvent;
use chrono::Utc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write_creds(path: &std::path::Path) {
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp_millis();
    std::fs::write(
        path,
        format!(
            r#"{{
              "openaiCodexOauth": {{
                "accessToken": "access-token",
                "refreshToken": "refresh-token",
                "expiresAt": {expires_at},
                "accountId": "acct_123"
              }}
            }}"#
        ),
    )
    .expect("write credentials");
}

#[tokio::test]
async fn provider_streams_codex_sse_as_archon_events() {
    let server = MockServer::start().await;
    let sse = [
        r#"data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress","model":"gpt-5.3-codex"}}"#,
        r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","status":"in_progress","role":"assistant","content":[]}}"#,
        r#"data: {"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hello"}"#,
        r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","model":"gpt-5.3-codex","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}"#,
        "data: [DONE]",
    ]
    .join("\n\n");

    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .and(header("authorization", "Bearer access-token"))
        .and(header("chatgpt-account-id", "acct_123"))
        .and(header("originator", "openclaw"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let cred_path = dir.path().join(".credentials.json");
    write_creds(&cred_path);
    let provider = CodexProvider::new_with_base_url(
        cred_path,
        SpoofConfig::default(),
        reqwest::Client::new(),
        server.uri(),
    )
    .expect("provider");

    let request = LlmRequest {
        model: "gpt-5.3-codex".into(),
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "say hello"}]
        })],
        ..LlmRequest::default()
    };
    let mut rx = provider.stream(request).await.expect("stream");
    let mut seen = Vec::new();
    while let Some(event) = rx.recv().await {
        seen.push(event);
    }

    assert!(matches!(seen[0], StreamEvent::MessageStart { .. }));
    assert!(
        seen.iter()
            .any(|event| matches!(event, StreamEvent::TextDelta { text, .. } if text == "hello"))
    );
    assert!(seen.iter().any(|event| matches!(
        event,
        StreamEvent::MessageDelta { usage: Some(usage), .. }
            if usage.input_tokens == 3 && usage.output_tokens == 2
    )));
    assert!(
        seen.iter()
            .any(|event| matches!(event, StreamEvent::MessageStop))
    );
}

#[tokio::test]
async fn provider_complete_preserves_tool_use_blocks() {
    let server = MockServer::start().await;
    let sse = format!(
        "{}\n\n",
        [
        r#"data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress","model":"gpt-5.3-codex"}}"#,
        r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","status":"in_progress","role":"assistant","content":[]}}"#,
        r#"data: {"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Need lookup."}"#,
        r#"data: {"type":"response.output_text.done","item_id":"msg_1","output_index":0,"content_index":0,"text":"Need lookup."}"#,
        r#"data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"Lookup","arguments":"","status":"in_progress"}}"#,
        r#"data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"{\"query\":\"arc"}"#,
        r#"data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"hon\"}"}"#,
        r#"data: {"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":1,"arguments":"{\"query\":\"archon\"}"}"#,
        r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","model":"gpt-5.3-codex","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}"#,
        "data: [DONE]",
        ]
        .join("\n\n")
    );

    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let cred_path = dir.path().join(".credentials.json");
    write_creds(&cred_path);
    let provider = CodexProvider::new_with_base_url(
        cred_path,
        SpoofConfig::default(),
        reqwest::Client::new(),
        server.uri(),
    )
    .expect("provider");

    let response = provider
        .complete(LlmRequest {
            model: "gpt-5.3-codex".into(),
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
            "id": "call_1",
            "name": "Lookup",
            "input": {"query": "archon"},
        })
    );
    assert_eq!(response.usage.input_tokens, 3);
    assert_eq!(response.usage.output_tokens, 2);
}

#[tokio::test]
async fn provider_maps_usage_limit_to_quota_exceeded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_string("usage limit reached"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let cred_path = dir.path().join(".credentials.json");
    write_creds(&cred_path);
    let provider = CodexProvider::new_with_base_url(
        cred_path,
        SpoofConfig::default(),
        reqwest::Client::new(),
        server.uri(),
    )
    .expect("provider");

    let err = provider
        .stream(LlmRequest::default())
        .await
        .expect_err("quota");
    assert!(
        matches!(err, archon_llm::provider::LlmError::QuotaExceeded(body) if body.contains("usage limit"))
    );
}
