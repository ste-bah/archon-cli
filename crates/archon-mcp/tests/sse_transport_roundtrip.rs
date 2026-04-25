//! TASK-P0-B-2A-WIRE (#197) — Full MCP SSE transport E2E roundtrip.
//!
//! Proves the hand-rolled SSE transport (classic MCP wire protocol) can:
//!   1. Open the SSE GET stream and discover the POST URL via `event: endpoint`
//!   2. POST a JSON-RPC `initialize` request and receive the response over SSE
//!   3. POST a `tools/list` request and decode the tools list from the SSE stream
//!
//! The test uses an in-process axum mock MCP SSE server that implements the
//! classic protocol:
//!   * GET `/sse` — opens the SSE stream, sends `event: endpoint` with POST URL
//!   * POST `/message` — parses JSON-RPC, returns 202 Accepted, pushes response
//!     back via the SSE stream as an `event: message` frame
//!
//! This is a wire-level roundtrip — no mocking of the transport or protocol.
//! Failure here = broken transport, not broken test.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};

use archon_mcp::lifecycle::connect_server_for_test;
use archon_mcp::types::ServerConfig;

// ---------------------------------------------------------------------------
// Mock MCP SSE server (classic protocol)
// ---------------------------------------------------------------------------

type FrameSender = mpsc::UnboundedSender<String>;
type FrameReceiver = Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>;

#[derive(Clone)]
struct MockState {
    tx: FrameSender,
    rx: FrameReceiver,
    post_path: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
}

/// GET /sse handler.
///
/// Sends an `event: endpoint` frame pointing at `/message?session=test` and
/// then streams any subsequent response frames pushed by the POST handler.
async fn sse_handler(
    State(state): State<MockState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    use futures_util::StreamExt;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    // Take the receiver; first-caller-wins. In a real MCP server each SSE
    // connection gets its own session; for a single-client test this is fine.
    let rx = {
        let mut guard = state.rx.lock().await;
        guard.take().expect("sse_handler called twice in one test")
    };

    let endpoint_data = state.post_path.clone();

    // First frame: event:endpoint with POST URL. Subsequent frames: event:message
    // carrying JSON-RPC response bodies from the POST handler.
    let endpoint_frame = futures_util::stream::once(async move {
        Ok(Event::default().event("endpoint").data(endpoint_data))
    });

    let response_frames = UnboundedReceiverStream::new(rx)
        .map(|body| Ok::<Event, Infallible>(Event::default().event("message").data(body)));

    let combined = endpoint_frame.chain(response_frames);

    Sse::new(combined).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)))
}

/// POST /message handler.
///
/// Parses the JSON-RPC request, builds a response, pushes it onto the SSE
/// stream via `state.tx`, and returns 202 Accepted with an empty body (per
/// the classic MCP SSE wire protocol).
async fn message_handler(
    State(state): State<MockState>,
    Query(_q): Query<SessionQuery>,
    Json(req): Json<serde_json::Value>,
) -> StatusCode {
    // Notifications carry no `id` and do not produce a response.
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED;
    };

    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let response = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mock-sse-server", "version": "0.1.0" }
            }
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echoes its input",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" }
                            },
                            "required": ["text"]
                        }
                    }
                ]
            }
        }),
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("method '{other}' not implemented in mock")
            }
        }),
    };

    let body = serde_json::to_string(&response).expect("serialize response");
    let _ = state.tx.send(body);
    StatusCode::ACCEPTED
}

/// Spin up the mock server on an ephemeral port. Returns (address, post_path).
async fn spawn_mock_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let state = MockState {
        tx,
        rx: Arc::new(Mutex::new(Some(rx))),
        post_path: "/message?session=test".into(),
    };

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .with_state(state);

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server)
}

// ---------------------------------------------------------------------------
// Test: full initialize + tools/list roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_transport_full_initialize_and_tools_list_roundtrip() {
    let (addr, server) = spawn_mock_server().await;

    let sse_url = format!("http://{addr}/sse");
    let config = ServerConfig {
        name: "sse-mock".into(),
        command: String::new(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "sse".into(),
        url: Some(sse_url),
        headers: None,
    };

    // connect_server_for_test is a thin pub-wrapper around the private
    // `connect_server` in lifecycle.rs that exposes it to integration tests.
    let client = tokio::time::timeout(Duration::from_secs(10), connect_server_for_test(&config))
        .await
        .expect("connect within 10s")
        .expect("initialize handshake succeeded");

    // tools/list roundtrip — proves the response frame cycle works post-init.
    let tools = tokio::time::timeout(Duration::from_secs(10), client.list_tools())
        .await
        .expect("list_tools within 10s")
        .expect("tools/list succeeded");

    assert_eq!(tools.len(), 1, "expected exactly the one mock tool");
    assert_eq!(tools[0].name, "echo");
    assert_eq!(tools[0].description.as_deref(), Some("Echoes its input"));

    // Schema should be surfaced through to the caller.
    let schema = &tools[0].input_schema;
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["text"].is_object());

    // Shutdown the client — cancels the rmcp service loop, drops the sink
    // which in turn drops the reqwest POST client and aborts the SSE stream.
    client.shutdown().await.expect("shutdown");

    server.abort();
}

// ---------------------------------------------------------------------------
// Test: endpoint URL resolution — absolute URL in `event: endpoint` works
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_transport_resolves_absolute_endpoint_url() {
    // Same mock but hand the server a post_path that is a FULL URL.
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let absolute = format!("http://{addr}/message?session=abs");

    let state = MockState {
        tx,
        rx: Arc::new(Mutex::new(Some(rx))),
        post_path: absolute,
    };
    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .with_state(state);

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });

    let sse_url = format!("http://{addr}/sse");
    let config = ServerConfig {
        name: "sse-mock-abs".into(),
        command: String::new(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "sse".into(),
        url: Some(sse_url),
        headers: None,
    };

    let client = tokio::time::timeout(Duration::from_secs(10), connect_server_for_test(&config))
        .await
        .expect("connect within 10s")
        .expect("initialize handshake succeeded with absolute endpoint");

    let tools = tokio::time::timeout(Duration::from_secs(5), client.list_tools())
        .await
        .expect("list_tools within 5s")
        .expect("tools/list succeeded");
    assert_eq!(tools.len(), 1);

    client.shutdown().await.expect("shutdown");
    server.abort();
}

// ---------------------------------------------------------------------------
// Test: missing url on transport=sse surfaces as a clean Transport error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_transport_without_url_errors_cleanly() {
    let config = ServerConfig {
        name: "sse-no-url".into(),
        command: String::new(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "sse".into(),
        url: None,
        headers: None,
    };

    let result = connect_server_for_test(&config).await;
    match result {
        Err(archon_mcp::types::McpError::Transport(msg)) => {
            assert!(
                msg.contains("sse") && msg.contains("url"),
                "error should mention sse+url, got: {msg}"
            );
        }
        Err(other) => panic!("expected Transport error, got {other}"),
        Ok(_) => panic!("expected error for missing url"),
    }
}
