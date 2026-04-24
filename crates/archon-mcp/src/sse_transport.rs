//! SSE (Server-Sent Events) framing primitives — **PARSER + STREAM READER ONLY**.
//!
//! This module provides:
//!   * [`SseFrame`] — a parsed SSE frame (event/data/id/retry fields)
//!   * [`create_sse_transport`] — opens a `reqwest` GET /events stream
//!   * [`SseTransport::connect_sse_stream`] — returns an `mpsc::Receiver<SseFrame>`
//!     of inbound frames parsed off the wire
//!
//! # Scope boundary (descoped from original #181)
//!
//! This is NOT a wire-ready MCP transport. It does NOT implement:
//!   * the POST /messages client→server direction
//!   * the `event: endpoint` frame handling that announces the POST URL
//!   * an `rmcp::transport::IntoTransport` adapter — so `McpClient::initialize`
//!     cannot consume an `SseTransport` as-is
//!   * a `"sse"` arm in `lifecycle.rs` transport dispatch
//!
//! Full wire-up is tracked under TASK-P0-B-2A-WIRE (#197), blocking
//! TASK-P0-B-2B-MCP-OAUTH (#182) which wraps a live transport.
//!
//! # Why descoped
//!
//! rmcp 1.3 removed its dedicated `SseClientTransport`. Re-implementing
//! the full bidirectional SSE MCP client from scratch is a standalone
//! ticket. The frame-parser primitives landed here are the reusable
//! building block; the POST channel + rmcp adapter + lifecycle dispatch
//! will land in #197.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use http::{HeaderName, HeaderValue};
use tokio::sync::mpsc;

use crate::types::McpError;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single parsed SSE frame (one `event:`/`data:`/`id:` block terminated by
/// a blank line).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseFrame {
    /// The `event:` field, if present. Absent or `"message"` denotes the
    /// default message type.
    pub event: Option<String>,
    /// The concatenated `data:` field(s). Multiple `data:` lines in one
    /// block are joined with `"\n"`, per the SSE spec.
    pub data: String,
    /// The `id:` field, if present.
    pub id: Option<String>,
    /// The server-suggested reconnection delay in milliseconds, if present.
    pub retry: Option<u64>,
}

/// SSE transport handle returned by [`create_sse_transport`].
///
/// The handle is lazy — no network activity occurs until
/// [`SseTransport::connect_sse_stream`] is called.
pub struct SseTransport {
    url: String,
    client: reqwest::Client,
    headers: HashMap<HeaderName, HeaderValue>,
}

impl SseTransport {
    /// URL of the SSE `/events` endpoint this transport will connect to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Open the SSE stream and return a receiver that yields parsed
    /// [`SseFrame`]s until the server closes the connection.
    ///
    /// This is a low-level primitive — it is a one-directional stream
    /// reader only. It does NOT handle the MCP POST /messages channel,
    /// the `event: endpoint` announcement frame, nor does it adapt into
    /// an `rmcp::transport::IntoTransport`. Full wire-up lives in #197.
    pub async fn connect_sse_stream(&self) -> Result<mpsc::Receiver<SseFrame>, McpError> {
        let mut req = self.client.get(&self.url);
        req = req.header(http::header::ACCEPT, "text/event-stream");
        for (name, value) in &self.headers {
            req = req.header(name.clone(), value.clone());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("SSE GET {} failed: {e}", self.url)))?;

        if !resp.status().is_success() {
            return Err(McpError::Transport(format!(
                "SSE GET {} returned status {}",
                self.url,
                resp.status()
            )));
        }

        let (tx, rx) = mpsc::channel::<SseFrame>(64);
        tokio::spawn(pump_sse_stream(resp, tx));
        Ok(rx)
    }
}

/// Background task that consumes the reqwest response byte stream, accumulates
/// bytes into UTF-8 lines, parses them into [`SseFrame`]s, and forwards the
/// frames to `tx`. Exits when the stream terminates or the receiver drops.
///
/// Exposed to the crate so the higher-level MCP SSE wire-up module
/// ([`crate::sse_mcp_transport`]) can reuse the same pump.
pub(crate) async fn pump_sse_stream(resp: reqwest::Response, tx: mpsc::Sender<SseFrame>) {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut current = SseFrameBuilder::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "SSE stream read error; terminating");
                return;
            }
        };
        buf.extend_from_slice(&chunk);

        // Drain any complete lines from `buf` (delimited by `\n`; strip a
        // trailing `\r` if present).
        loop {
            let Some(nl) = buf.iter().position(|&b| b == b'\n') else {
                break;
            };
            let mut line_bytes = buf.drain(..=nl).collect::<Vec<u8>>();
            line_bytes.pop(); // remove '\n'
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "non-UTF8 SSE line; skipping");
                    continue;
                }
            };

            if line.is_empty() {
                // Blank line = frame dispatch.
                if let Some(frame) = current.take_frame() {
                    if tx.send(frame).await.is_err() {
                        return; // receiver dropped
                    }
                }
                continue;
            }
            if line.starts_with(':') {
                // SSE comment line; ignore.
                continue;
            }
            current.ingest_line(line);
        }
    }

    // Stream ended. If there's a dangling frame without a trailing blank line,
    // dispatch it anyway (lenient parsing matches what rmcp does internally).
    if let Some(frame) = current.take_frame() {
        let _ = tx.send(frame).await;
    }
}

/// Incremental builder for a single SSE frame.
///
/// Exposed to the crate so [`crate::sse_reconnect`] can reuse the exact
/// same line-parsing semantics without duplicating them.
#[derive(Default)]
pub(crate) struct SseFrameBuilder {
    event: Option<String>,
    data: Vec<String>,
    id: Option<String>,
    retry: Option<u64>,
    dirty: bool,
}

impl SseFrameBuilder {
    pub(crate) fn ingest_line(&mut self, line: &str) {
        // Field / value split: first ':', then optional single space.
        let (field, value) = match line.find(':') {
            Some(idx) => {
                let (f, rest) = line.split_at(idx);
                let v = &rest[1..];
                let v = v.strip_prefix(' ').unwrap_or(v);
                (f, v)
            }
            None => (line, ""), // field-only line per SSE spec
        };
        self.dirty = true;
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            "id" => self.id = Some(value.to_string()),
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.retry = Some(ms);
                }
            }
            _ => {
                // Unknown field; per spec, ignore.
            }
        }
    }

    pub(crate) fn take_frame(&mut self) -> Option<SseFrame> {
        if !self.dirty {
            return None;
        }
        let data = self.data.join("\n");
        let frame = SseFrame {
            event: self.event.take(),
            data,
            id: self.id.take(),
            retry: self.retry.take(),
        };
        self.data.clear();
        self.dirty = false;
        Some(frame)
    }
}

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

/// Create an SSE framing primitive handle for reading a remote
/// `text/event-stream` endpoint.
///
/// Builds a `reqwest::Client` with the given `connect_timeout` and optional
/// custom headers (e.g. `Authorization`), and records the target URL. The
/// returned [`SseTransport`] is lazy — no connection is established until
/// [`SseTransport::connect_sse_stream`] is called.
///
/// Note: this is NOT a wire-ready MCP transport. See the module-level
/// scope-boundary doc. Full MCP SSE transport wire-up (POST channel +
/// `rmcp::IntoTransport` adapter) is tracked under #197.
pub fn create_sse_transport(
    url: &str,
    headers: Option<&HashMap<String, String>>,
    connect_timeout: Duration,
) -> Result<SseTransport, McpError> {
    let client = reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| McpError::Transport(format!("failed to build HTTP client: {e}")))?;

    let mut converted: HashMap<HeaderName, HeaderValue> = HashMap::new();

    if let Some(hdrs) = headers {
        for (name, value) in hdrs {
            let header_name: HeaderName = name
                .parse()
                .map_err(|e| McpError::Transport(format!("invalid header name '{name}': {e}")))?;
            let header_value: HeaderValue = value.parse().map_err(|e| {
                McpError::Transport(format!("invalid header value for '{name}': {e}"))
            })?;
            converted.insert(header_name, header_value);
        }
    }

    Ok(SseTransport {
        url: url.to_string(),
        client,
        headers: converted,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_sse_transport_with_url_succeeds() {
        let transport =
            create_sse_transport("http://localhost:9/events", None, Duration::from_secs(5))
                .expect("construct SSE transport");
        assert_eq!(transport.url(), "http://localhost:9/events");
    }

    #[tokio::test]
    async fn create_sse_transport_with_headers_succeeds() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token".into());
        headers.insert("X-Api-Key".into(), "key123".into());

        let transport = create_sse_transport(
            "http://localhost:9/events",
            Some(&headers),
            Duration::from_secs(5),
        )
        .expect("construct SSE transport with headers");
        assert_eq!(transport.headers.len(), 2);
    }

    #[test]
    fn create_sse_transport_invalid_header_name_errors() {
        let mut headers = HashMap::new();
        headers.insert("Invalid Header\nName".into(), "value".into());
        let result = create_sse_transport(
            "http://localhost:9/events",
            Some(&headers),
            Duration::from_secs(5),
        );
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid header name"), "msg was: {msg}");
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn create_sse_transport_invalid_header_value_errors() {
        let mut headers = HashMap::new();
        headers.insert("X-Bad".into(), "value\0with\0nulls".into());
        let result = create_sse_transport(
            "http://localhost:9/events",
            Some(&headers),
            Duration::from_secs(5),
        );
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid header value"), "msg was: {msg}");
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn sse_frame_builder_parses_simple_data_line() {
        let mut b = SseFrameBuilder::default();
        b.ingest_line("data: hello");
        let frame = b.take_frame().expect("frame");
        assert_eq!(frame.data, "hello");
        assert!(frame.event.is_none());
        assert!(frame.id.is_none());
    }

    #[test]
    fn sse_frame_builder_joins_multiline_data() {
        let mut b = SseFrameBuilder::default();
        b.ingest_line("data: line1");
        b.ingest_line("data: line2");
        let frame = b.take_frame().expect("frame");
        assert_eq!(frame.data, "line1\nline2");
    }

    #[test]
    fn sse_frame_builder_takes_event_id_retry() {
        let mut b = SseFrameBuilder::default();
        b.ingest_line("event: ping");
        b.ingest_line("id: 42");
        b.ingest_line("retry: 1500");
        b.ingest_line("data: {}");
        let frame = b.take_frame().expect("frame");
        assert_eq!(frame.event.as_deref(), Some("ping"));
        assert_eq!(frame.id.as_deref(), Some("42"));
        assert_eq!(frame.retry, Some(1500));
        assert_eq!(frame.data, "{}");
    }

    // ----- End-to-end handshake test (Gate 5 evidence) -----

    #[tokio::test]
    async fn sse_transport_e2e_handshake_mock() {
        use axum::Router;
        use axum::response::sse::{Event, Sse as AxumSse};
        use axum::routing::get;
        use futures_util::stream;
        use std::convert::Infallible;
        use std::net::SocketAddr;
        use tokio::net::TcpListener;

        // 1. Spin up an in-process axum SSE echo server on a random port.
        //
        //    The handler emits exactly ONE `data:` frame containing a
        //    JSON-RPC notification, then closes the stream. This proves the
        //    wire protocol round-trip (client opens GET, server emits SSE,
        //    client parses the frame) without any mocking.
        let app = Router::new().route(
            "/events",
            get(|| async {
                let payload = r#"{"jsonrpc":"2.0","method":"ping"}"#;
                let events =
                    stream::iter(vec![Ok::<_, Infallible>(Event::default().data(payload))]);
                AxumSse::new(events)
            }),
        );

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind random port");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum serve");
        });

        // 2. Real client — create_sse_transport + connect_sse_stream + recv.
        let url = format!("http://{}/events", addr);
        let transport =
            create_sse_transport(&url, None, Duration::from_secs(5)).expect("construct transport");

        let mut rx = transport
            .connect_sse_stream()
            .await
            .expect("connect SSE stream");

        let frame = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("did not receive any SSE frame within 3s")
            .expect("channel closed before any frame arrived");

        // 3. Assert wire-level frame shape — real SSE bytes parsed, not mocked.
        assert!(
            frame.data.contains(r#""method":"ping""#),
            "unexpected frame data: {:?}",
            frame.data
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&frame.data).expect("data is valid JSON");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["method"], "ping");

        // 4. Stream closes; further recvs return None.
        let trailing = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        // We don't strictly require None here (server may keep SSE alive);
        // a timeout or None both indicate the single-frame exchange is done.
        match trailing {
            Ok(None) | Err(_) => {}
            Ok(Some(extra)) => panic!("unexpected extra frame: {:?}", extra),
        }

        server.abort();
    }
}
