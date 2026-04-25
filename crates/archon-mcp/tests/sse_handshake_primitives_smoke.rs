//! TASK-P1-7 (#192) — MCP SSE handshake PRIMITIVES smoke (Option A descope).
//!
//! Proves the SSE frame-parser primitives from #181 correctly decode
//! SSE frames carrying real MCP JSON-RPC 2.0 initialize/initialized
//! request-response shapes. This is PRIMITIVES-LEVEL verification only.
//!
//! Full handshake (including POST /messages client-to-server direction,
//! event:endpoint frame handling, rmcp IntoTransport adapter, lifecycle
//! dispatch wire) is deferred to TASK-P0-B-2A-WIRE (#197). The blocker
//! is that rmcp 1.3 removed its dedicated SseClientTransport; wire-up
//! requires a standalone bidirectional SSE MCP client that is a
//! ticket-sized effort on its own.
//!
//! This test proves the parser is ready to be consumed by #197.

use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::{StreamExt, stream};
use tokio::net::TcpListener;

use archon_mcp::sse_transport::create_sse_transport;

/// Minimal MCP initialize REQUEST shape per JSON-RPC 2.0 + MCP spec.
const MCP_INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"archon-cli","version":"0.1"}}}"#;

/// Minimal MCP initialize RESPONSE shape.
const MCP_INITIALIZE_RESPONSE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock-server","version":"0.1"}}}"#;

/// Minimal MCP initialized NOTIFICATION shape.
const MCP_INITIALIZED_NOTIFICATION: &str =
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

async fn handshake_handler()
-> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let frames = vec![
        Ok(Event::default().data(MCP_INITIALIZE_REQUEST)),
        Ok(Event::default().data(MCP_INITIALIZE_RESPONSE)),
        Ok(Event::default().data(MCP_INITIALIZED_NOTIFICATION)),
    ];
    Sse::new(stream::iter(frames).chain(stream::pending()))
        .keep_alive(KeepAlive::new().interval(Duration::from_millis(100)))
}

#[tokio::test]
async fn sse_frame_parser_decodes_mcp_initialize_request_response_notification() {
    // Spin mock SSE server.
    let app: Router = Router::new().route("/mcp/events", get(handshake_handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Connect client via the #181 primitives.
    let url = format!("http://{addr}/mcp/events");
    let transport = create_sse_transport(
        &url,
        None::<&HashMap<String, String>>,
        Duration::from_secs(5),
    )
    .expect("create transport");
    let mut rx = transport
        .connect_sse_stream()
        .await
        .expect("connect stream");

    // Collect 3 frames (bounded by 5s).
    let mut frames = Vec::new();
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => panic!("stream ended early after {} frames", frames.len()),
            Err(_) => panic!("timeout waiting for frame {}", frames.len() + 1),
        }
    }

    assert_eq!(frames.len(), 3);

    // Verify each frame's data round-trips through the parser byte-for-byte.
    assert_eq!(frames[0].data, MCP_INITIALIZE_REQUEST);
    assert_eq!(frames[1].data, MCP_INITIALIZE_RESPONSE);
    assert_eq!(frames[2].data, MCP_INITIALIZED_NOTIFICATION);

    // Verify JSON shape of each frame is real MCP payload.
    let req: serde_json::Value = serde_json::from_str(&frames[0].data).expect("req parses");
    assert_eq!(req["jsonrpc"], "2.0");
    assert_eq!(req["method"], "initialize");
    assert_eq!(req["params"]["protocolVersion"], "2024-11-05");
    assert_eq!(req["params"]["clientInfo"]["name"], "archon-cli");

    let resp: serde_json::Value = serde_json::from_str(&frames[1].data).expect("resp parses");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert!(resp["result"].is_object());
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");

    let notif: serde_json::Value = serde_json::from_str(&frames[2].data).expect("notif parses");
    assert_eq!(notif["jsonrpc"], "2.0");
    assert_eq!(notif["method"], "notifications/initialized");

    server.abort();
}
