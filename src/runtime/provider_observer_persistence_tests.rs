use super::*;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::providers::AnthropicProvider;
use archon_llm::types::Secret;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn real_anthropic_provider(url: String) -> Arc<dyn LlmProvider> {
    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "session-test".into(),
        "device-test".into(),
        String::new(),
    );
    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        identity,
        Some(url),
    );
    Arc::new(AnthropicProvider::new(client))
}

async fn accept_anthropic_request(
    listener: &TcpListener,
) -> (tokio::net::TcpStream, serde_json::Value) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.unwrap();
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap();
    while request.len() - header_end < content_length {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.unwrap();
        request.extend_from_slice(&buffer[..read]);
    }
    let body = serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
    (socket, body)
}

async fn serve_anthropic_sse(
    listener: TcpListener,
    captured: tokio::sync::oneshot::Sender<serde_json::Value>,
) {
    let (mut socket, request) = accept_anthropic_request(&listener).await;
    captured.send(request).unwrap();
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-real\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":11,\"cache_creation_input_tokens\":3,\"cache_read_input_tokens\":5}}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
        "event: message_stop\n",
        "data: {}\n\n"
    );
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
}

#[tokio::test]
async fn anthropic_compatible_proxy_sse_strips_cache_marker_and_persists_usage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("learning.db");
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().unwrap(),
        "open observed learning db",
    )
    .unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (captured_tx, captured_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(serve_anthropic_sse(listener, captured_tx));
    let observed = ObservedLlmProvider::new(
        real_anthropic_provider(url),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;
    let mut stream = observed
        .stream(LlmRequest {
            model: "claude-sonnet-4-6".into(),
            messages: vec![serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "latest",
                    "cache_control": {"type": "ephemeral"}
                }]
            })],
            extra: serde_json::json!({"archon_runtime": {
                "run_id": "run-real", "session_id": "session-real", "turn": 1,
                "round": 2, "effective_denominator": 100
            }}),
            ..LlmRequest::default()
        })
        .await
        .unwrap();
    while stream.recv().await.is_some() {}
    let captured_body = captured_rx.await.unwrap();
    assert_eq!(
        captured_body["messages"][0]["content"][0].get("cache_control"),
        None
    );
    drop(observed);
    drop(db);

    let reopened = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().unwrap(),
        "reopen observed learning db",
    )
    .unwrap();
    let rows = archon_learning::llm_call_usage::list_llm_call_usage(
        &reopened,
        &archon_learning::llm_call_usage::LlmCallUsageScope::new(
            Some("run-real"),
            Some("session-real"),
        ),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].terminal_status, "succeeded");
    assert_eq!(
        rows[0].input_tokens,
        archon_learning::llm_call_usage::UsageAvailability::Known(11)
    );
    assert_eq!(
        rows[0].cache_creation_input_tokens,
        archon_learning::llm_call_usage::UsageAvailability::Known(3)
    );
    assert_eq!(
        rows[0].cache_read_input_tokens,
        archon_learning::llm_call_usage::UsageAvailability::Known(5)
    );
    assert_eq!(
        rows[0].output_tokens,
        archon_learning::llm_call_usage::UsageAvailability::Known(7)
    );
}
