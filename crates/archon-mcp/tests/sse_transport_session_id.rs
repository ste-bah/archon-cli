//! TASK-203 MCP-SSE-SESSION-ID — Mcp-Session-Id header coordination.
//!
//! Proves the SSE MCP transport:
//!   1. Parses `Mcp-Session-Id: <id>` from the first POST response headers
//!      (classic MCP servers return it as part of the `initialize` 202).
//!   2. Injects `Mcp-Session-Id: <cached>` on every subsequent POST.
//!   3. On server 404 (session invalid), logs + drops the message and
//!      surfaces to the caller as a bounded rmcp request timeout — NOT an
//!      infinite retry loop.
//!   4. Maintains per-client isolation: two concurrent `connect_mcp` calls
//!      against distinct servers see distinct, non-interfering session IDs.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};

use archon_mcp::lifecycle::connect_server_for_test;
use archon_mcp::types::{McpError, ServerConfig};

// ---------------------------------------------------------------------------
// Mock server with Mcp-Session-Id coordination
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SessionMockState {
    tx: mpsc::UnboundedSender<String>,
    rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    /// Session ID this mock issues on the first POST response and then
    /// requires on every subsequent POST.
    issued_session: String,
    /// Per-POST header observed (captured so tests can assert).
    observed_session_headers: Arc<Mutex<Vec<Option<String>>>>,
    post_path: String,
    /// Count of POSTs received (used to tell initialize from later requests).
    post_count: Arc<AtomicU32>,
    /// If true, every POST returns 404 regardless of Mcp-Session-Id.
    always_404: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
}

async fn session_sse(
    State(state): State<SessionMockState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    use tokio_stream::wrappers::UnboundedReceiverStream;

    let rx = state
        .rx
        .lock()
        .await
        .take()
        .expect("session_sse called twice");

    let endpoint_data = state.post_path.clone();
    let endpoint_frame = futures_util::stream::once(async move {
        Ok(Event::default().event("endpoint").data(endpoint_data))
    });
    let responses = UnboundedReceiverStream::new(rx)
        .map(|body| Ok::<Event, Infallible>(Event::default().event("message").data(body)));
    Sse::new(endpoint_frame.chain(responses))
}

async fn session_message(
    State(state): State<SessionMockState>,
    Query(_q): Query<SessionQuery>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> (StatusCode, HeaderMap) {
    // Always record the observed Mcp-Session-Id header.
    let observed = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    state.observed_session_headers.lock().await.push(observed.clone());

    if state.always_404 {
        return (StatusCode::NOT_FOUND, HeaderMap::new());
    }

    let post_n = state.post_count.fetch_add(1, Ordering::SeqCst);

    // First POST is initialize. Respond with Mcp-Session-Id header in the
    // 202 response. Later POSTs must carry that header or 404.
    if post_n > 0
        && observed.as_deref() != Some(state.issued_session.as_str())
    {
        return (StatusCode::NOT_FOUND, HeaderMap::new());
    }

    let Some(id) = req.get("id").cloned() else {
        // Notifications: no response body expected via SSE.
        let mut resp_headers = HeaderMap::new();
        if post_n == 0 {
            resp_headers.insert(
                "mcp-session-id",
                HeaderValue::from_str(&state.issued_session).unwrap(),
            );
        }
        return (StatusCode::ACCEPTED, resp_headers);
    };

    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let resp = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "session-mock", "version": "0.1.0" }
            }
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": [{
                "name": "noop",
                "description": "No-op tool",
                "inputSchema": { "type": "object", "properties": {} }
            }]}
        }),
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("unsupported '{other}'")
            }
        }),
    };
    let _ = state.tx.send(serde_json::to_string(&resp).unwrap());

    let mut resp_headers = HeaderMap::new();
    if post_n == 0 {
        // Issue the session ID on the first POST (the initialize response).
        resp_headers.insert(
            "mcp-session-id",
            HeaderValue::from_str(&state.issued_session).unwrap(),
        );
    }
    (StatusCode::ACCEPTED, resp_headers)
}

async fn spawn_session_mock(
    issued: &str,
    always_404: bool,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    SessionMockState,
) {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let state = SessionMockState {
        tx,
        rx: Arc::new(Mutex::new(Some(rx))),
        issued_session: issued.into(),
        observed_session_headers: Arc::new(Mutex::new(Vec::new())),
        post_path: "/message?session=irrelevant".into(),
        post_count: Arc::new(AtomicU32::new(0)),
        always_404,
    };
    let app = Router::new()
        .route("/sse", get(session_sse))
        .route("/message", post(session_message))
        .with_state(state.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server, state)
}

fn sse_config(name: &str, addr: SocketAddr) -> ServerConfig {
    ServerConfig {
        name: name.into(),
        command: String::new(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "sse".into(),
        url: Some(format!("http://{addr}/sse")),
        headers: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: Session-ID parsed from initialize response + injected on next POST
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_id_parsed_from_initialize_and_injected_on_next_post() {
    let (addr, server, state) = spawn_session_mock("sess-abc-123", false).await;

    let config = sse_config("session-mock", addr);
    let client = tokio::time::timeout(
        Duration::from_secs(10),
        connect_server_for_test(&config),
    )
    .await
    .expect("connect within 10s")
    .expect("initialize succeeded");

    // tools/list — second POST. Must carry Mcp-Session-Id: sess-abc-123.
    let tools = tokio::time::timeout(Duration::from_secs(5), client.list_tools())
        .await
        .expect("list_tools within 5s")
        .expect("tools/list succeeded");
    assert_eq!(tools.len(), 1);

    // Inspect captured headers — server should have seen:
    //   POST #1 (initialize): no Mcp-Session-Id yet.
    //   POST #2 (notifications/initialized): header present.
    //   POST #3 (tools/list): header present.
    let observed = state.observed_session_headers.lock().await.clone();
    assert!(observed.len() >= 2, "expected ≥2 POSTs, saw {}", observed.len());
    assert_eq!(
        observed[0], None,
        "initialize POST should not carry a session header yet"
    );
    for h in observed.iter().skip(1) {
        assert_eq!(
            h.as_deref(),
            Some("sess-abc-123"),
            "subsequent POST missing expected session header"
        );
    }

    client.shutdown().await.expect("shutdown");
    server.abort();
}

// ---------------------------------------------------------------------------
// Test 2: Persistent 404 surfaces as bounded error — no infinite retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn persistent_404_surfaces_bounded_error() {
    // always_404=true — every POST returns 404 regardless of session header.
    // rmcp initialize will fail because no response frame ever arrives.
    let (addr, server, _state) = spawn_session_mock("sess-404", true).await;
    let config = sse_config("session-404", addr);

    // Bound the whole call at 40s — slightly above rmcp's INIT_TIMEOUT (30s)
    // so the transport's own bounded error surfaces first. If the transport
    // were to infinite-loop on 404s, this outer timeout catches it.
    let result = tokio::time::timeout(
        Duration::from_secs(40),
        connect_server_for_test(&config),
    )
    .await;

    match result {
        Err(_) => panic!("connect should have bounded-errored, not timed out the test harness"),
        Ok(Ok(_client)) => panic!("connect should have errored — server returns 404 for everything"),
        Ok(Err(e)) => {
            // Any McpError is acceptable; the point is bounded time.
            let _e: McpError = e;
        }
    }

    server.abort();
}

// ---------------------------------------------------------------------------
// Test 3: Per-client isolation — two concurrent transports see distinct IDs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn per_client_session_isolation() {
    let (addr_a, server_a, state_a) = spawn_session_mock("SESSION-A", false).await;
    let (addr_b, server_b, state_b) = spawn_session_mock("SESSION-B", false).await;

    let (ca, cb) = tokio::join!(
        async {
            let cfg = sse_config("client-a", addr_a);
            let c = connect_server_for_test(&cfg).await.expect("A connect");
            c.list_tools().await.expect("A tools/list");
            c
        },
        async {
            let cfg = sse_config("client-b", addr_b);
            let c = connect_server_for_test(&cfg).await.expect("B connect");
            c.list_tools().await.expect("B tools/list");
            c
        },
    );

    // Each server should have observed its own session header injected,
    // never the other's.
    let obs_a = state_a.observed_session_headers.lock().await.clone();
    let obs_b = state_b.observed_session_headers.lock().await.clone();

    for h in obs_a.iter().skip(1).flatten() {
        assert_eq!(h.as_str(), "SESSION-A", "client A leaked session: {h}");
        assert_ne!(h.as_str(), "SESSION-B");
    }
    for h in obs_b.iter().skip(1).flatten() {
        assert_eq!(h.as_str(), "SESSION-B", "client B leaked session: {h}");
        assert_ne!(h.as_str(), "SESSION-A");
    }

    ca.shutdown().await.expect("A shutdown");
    cb.shutdown().await.expect("B shutdown");
    server_a.abort();
    server_b.abort();
}
