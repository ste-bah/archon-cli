//! Full bidirectional MCP transport over classic Server-Sent Events (#197).
//!
//! Layers on top of the one-way SSE frame-parser primitives in
//! [`crate::sse_transport`] to implement the complete MCP SSE wire protocol:
//!
//! ```text
//! Client                                 Server
//!   |  GET /sse  (Accept: text/event-stream) |
//!   |--------------------------------------->|
//!   |                                        |
//!   |   event: endpoint                      |
//!   |   data: /message?session=<id>          |
//!   |<---------------------------------------|
//!   |                                        |
//!   |  POST /message?session=<id>            |
//!   |    body: <JSON-RPC request>            |
//!   |--------------------------------------->|
//!   |   HTTP 202 Accepted (empty)            |
//!   |<---------------------------------------|
//!   |                                        |
//!   |   event: message                       |
//!   |   data: <JSON-RPC response>            |
//!   |<---------------------------------------|
//! ```
//!
//! The returned `(Sink, Stream)` tuple is consumed by rmcp's blanket
//! `IntoTransport` impl, so this transport is a drop-in for the other
//! transports (`stdio`, `http`, `ws`) dispatched by
//! [`crate::lifecycle::connect_server`].
//!
//! # Scope
//!
//! Ships the happy path: endpoint discovery, frame parse, POST, JSON-RPC
//! roundtrip. Since #202 MCP-SSE-HARDEN-RETRY landed, the inbound SSE pump
//! also auto-reconnects on stream drop with `Last-Event-ID` replay and
//! bounded exponential backoff (see [`crate::sse_reconnect`]).
//!
//! Still deferred to follow-up tickets:
//!
//! * `Mcp-Session-Id` header coordination — #203 MCP-SSE-SESSION-ID
//!
//! POST failures are logged at `tracing::warn!` but do not fail the transport;
//! rmcp observes request timeouts for any messages that fail to deliver.

use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{Sink, Stream, StreamExt};
use http::{HeaderName, HeaderValue};
use rmcp::service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

use crate::sse_reconnect::{ReconnectConfig, pump_sse_stream_with_reconnect};
use crate::sse_transport::SseFrame;
use crate::types::McpError;

/// Boxed, pinned MCP SSE inbound stream — yields deserialized JSON-RPC messages.
pub type SseMcpStream =
    Pin<Box<dyn Stream<Item = RxJsonRpcMessage<RoleClient>> + Send + 'static>>;

/// Boxed, pinned MCP SSE outbound sink — POSTs each JSON-RPC message to the
/// endpoint URL discovered during the `event: endpoint` handshake.
pub type SseMcpSink = Pin<
    Box<dyn Sink<TxJsonRpcMessage<RoleClient>, Error = McpError> + Send + 'static>,
>;

/// Default timeout for the `event: endpoint` discovery handshake.
///
/// The server must emit the endpoint frame before this expires, otherwise
/// [`connect_mcp`] errors. Deliberately short — `endpoint` is the first frame
/// a spec-compliant server sends.
pub const ENDPOINT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Buffer size for both the inbound SSE frame channel and the outbound POST
/// channel.
///
/// 64 is generous for MCP request/response traffic — a `tools/list` response
/// with 100+ tools fits in a single frame.
pub(crate) const SSE_CHANNEL_BUFFER: usize = 64;

/// Successful output of [`setup_sse_inbound`] — the pieces needed to build
/// the inbound stream and any outbound sink (default or auth-wrapping).
///
/// Exposed `pub(crate)` so the OAuth wrapper in [`crate::sse_oauth_transport`]
/// can reuse the SSE handshake logic without duplicating it.
pub(crate) struct SseInboundSetup {
    /// HTTP client used to open the SSE stream (reused for POSTs by callers).
    pub http_client: reqwest::Client,
    /// Absolute POST URL discovered via `event: endpoint`.
    pub post_url: Url,
    /// Parsed custom headers (already validated) — caller-supplied.
    pub header_map: HashMap<HeaderName, HeaderValue>,
    /// Receiver that yields SSE frames AFTER the endpoint frame has been consumed.
    pub frame_rx: mpsc::Receiver<SseFrame>,
}

/// Open the SSE GET stream, wait for the `event: endpoint` frame, and return
/// the pieces both the vanilla and OAuth-wrapped transports need.
///
/// This is the steps-1-through-6 portion of [`connect_mcp`] extracted so the
/// OAuth wrapper can reuse it — the OAuth wrapper differs only in the POST
/// pump (auth header + 401 refresh), not in the SSE handshake.
pub(crate) async fn setup_sse_inbound(
    sse_url: &str,
    headers: Option<&HashMap<String, String>>,
    connect_timeout: Duration,
) -> Result<SseInboundSetup, McpError> {
    // 1. Validate + parse the SSE URL.
    let sse_url_parsed = Url::parse(sse_url)
        .map_err(|e| McpError::Transport(format!("sse: invalid URL '{sse_url}': {e}")))?;

    // 2. Build HTTP client.
    let http_client = reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| McpError::Transport(format!("sse: build HTTP client: {e}")))?;

    let header_map = convert_headers(headers)?;

    // 3. Spawn the reconnecting SSE pump. It opens the initial GET internally
    //    AND transparently handles Last-Event-ID replay on stream drop
    //    (#202 MCP-SSE-HARDEN-RETRY). If the initial connect + all retries
    //    fail, the endpoint-frame timeout below will surface the error.
    let (frame_tx, mut frame_rx) = mpsc::channel::<SseFrame>(SSE_CHANNEL_BUFFER);
    tokio::spawn(pump_sse_stream_with_reconnect(
        http_client.clone(),
        sse_url.to_string(),
        header_map.clone(),
        frame_tx,
        ReconnectConfig::default(),
    ));

    // 5. Consume frames until `event: endpoint` arrives.
    let endpoint_frame = tokio::time::timeout(ENDPOINT_HANDSHAKE_TIMEOUT, async {
        loop {
            match frame_rx.recv().await {
                None => {
                    return Err(McpError::Transport(
                        "sse: stream closed before endpoint frame arrived".into(),
                    ));
                }
                Some(frame) => {
                    if frame.event.as_deref() == Some("endpoint") {
                        return Ok(frame);
                    }
                    tracing::debug!(event = ?frame.event, "sse: pre-endpoint frame ignored");
                }
            }
        }
    })
    .await
    .map_err(|_| {
        McpError::Transport(format!(
            "sse: endpoint frame not received within {ENDPOINT_HANDSHAKE_TIMEOUT:?}"
        ))
    })??;

    // 6. Resolve POST URL.
    let post_url_str = endpoint_frame.data.trim().to_string();
    if post_url_str.is_empty() {
        return Err(McpError::Transport(
            "sse: endpoint frame had empty data field".into(),
        ));
    }
    let post_url = sse_url_parsed.join(&post_url_str).map_err(|e| {
        McpError::Transport(format!("sse: invalid endpoint URL '{post_url_str}': {e}"))
    })?;

    tracing::info!(
        sse = %sse_url_parsed,
        post = %post_url,
        "sse: MCP transport ready"
    );

    Ok(SseInboundSetup {
        http_client,
        post_url,
        header_map,
        frame_rx,
    })
}

/// Wrap a post-handshake SSE frame receiver into the boxed inbound JSON-RPC
/// message stream consumed by rmcp.
///
/// Exposed `pub(crate)` so the OAuth wrapper can build the same stream shape
/// without duplicating the filter/parse logic.
pub(crate) fn build_inbound_stream(frame_rx: mpsc::Receiver<SseFrame>) -> SseMcpStream {
    let stream = ReceiverStream::new(frame_rx).filter_map(|frame: SseFrame| async move {
        match frame.event.as_deref() {
            None | Some("message") => {}
            Some(other) => {
                tracing::trace!(event = %other, "sse: non-message frame skipped");
                return None;
            }
        }
        if frame.data.is_empty() {
            return None;
        }
        match serde_json::from_str::<RxJsonRpcMessage<RoleClient>>(&frame.data) {
            Ok(msg) => Some(msg),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    data = %frame.data,
                    "sse: failed to parse JSON-RPC payload from SSE frame"
                );
                None
            }
        }
    });
    Box::pin(stream)
}

/// Open a bidirectional MCP transport over classic Server-Sent Events.
///
/// Steps:
///   1. `GET sse_url` with `Accept: text/event-stream` — opens persistent stream.
///   2. Waits for the first `event: endpoint` frame (bounded by
///      [`ENDPOINT_HANDSHAKE_TIMEOUT`]). The `data` field is the URL the
///      client POSTs JSON-RPC requests to (absolute or relative to `sse_url`).
///   3. Returns a `(Sink, Stream)` tuple that rmcp's blanket `IntoTransport`
///      impl consumes as a client transport.
///
/// Frame filtering on the inbound stream:
///   * `event: endpoint` (only at init) — consumed, never reaches the caller
///   * `event: message` or default (no event field) — parsed as JSON-RPC
///   * any other named event (e.g. `ping`) — skipped
///   * comment lines (`:`) — already skipped by the frame parser
pub async fn connect_mcp(
    sse_url: &str,
    headers: Option<&HashMap<String, String>>,
    connect_timeout: Duration,
) -> Result<(SseMcpSink, SseMcpStream), McpError> {
    let SseInboundSetup {
        http_client,
        post_url,
        header_map,
        frame_rx,
    } = setup_sse_inbound(sse_url, headers, connect_timeout).await?;

    let rx_stream_boxed = build_inbound_stream(frame_rx);

    // Outbound Sink: POST each message to the endpoint URL via a background
    // task. The Sink side is a `futures_util::sink::unfold` over a tokio mpsc
    // Sender; `Box::pin` gives it `Unpin + Send + 'static` so rmcp's
    // `IntoTransport` blanket impl accepts it.
    let (tx_chan, tx_rx) = mpsc::channel::<TxJsonRpcMessage<RoleClient>>(SSE_CHANNEL_BUFFER);
    tokio::spawn(post_pump_task(http_client, post_url, header_map, tx_rx));

    let sink = futures_util::sink::unfold(
        tx_chan,
        |tx, msg: TxJsonRpcMessage<RoleClient>| async move {
            tx.send(msg)
                .await
                .map_err(|e| McpError::Transport(format!("sse: outbound channel closed: {e}")))?;
            Ok::<_, McpError>(tx)
        },
    );
    let sink_boxed: SseMcpSink = Box::pin(sink);

    Ok((sink_boxed, rx_stream_boxed))
}

/// Background task that drains the outbound channel and POSTs each message
/// to the MCP endpoint URL.
///
/// Errors are logged but do not stop the pump: the rmcp runtime will observe
/// request timeouts for messages whose responses never arrive.
pub(crate) async fn post_pump_task(
    client: reqwest::Client,
    url: Url,
    headers: HashMap<HeaderName, HeaderValue>,
    mut rx: mpsc::Receiver<TxJsonRpcMessage<RoleClient>>,
) {
    while let Some(msg) = rx.recv().await {
        let body = match serde_json::to_string(&msg) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "sse: failed to serialize outgoing JSON-RPC");
                continue;
            }
        };
        let mut post = client
            .post(url.as_str())
            .body(body)
            .header(http::header::CONTENT_TYPE, "application/json");
        for (name, value) in &headers {
            post = post.header(name.clone(), value.clone());
        }
        match post.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::trace!(status = %resp.status(), "sse: POST ok");
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "sse: POST non-2xx");
            }
            Err(e) => {
                tracing::warn!(error = %e, "sse: POST failed");
            }
        }
    }
    tracing::debug!("sse: outbound POST pump exited (sink closed)");
}

/// Convert the user-supplied header map into validated
/// `http::HeaderName`/`HeaderValue` pairs.
pub(crate) fn convert_headers(
    headers: Option<&HashMap<String, String>>,
) -> Result<HashMap<HeaderName, HeaderValue>, McpError> {
    let mut out = HashMap::new();
    if let Some(h) = headers {
        for (n, v) in h {
            let name: HeaderName = n.parse().map_err(|e| {
                McpError::Transport(format!("sse: invalid header name '{n}': {e}"))
            })?;
            let value: HeaderValue = v.parse().map_err(|e| {
                McpError::Transport(format!("sse: invalid header value for '{n}': {e}"))
            })?;
            out.insert(name, value);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_mcp_invalid_url_errors() {
        let result = connect_mcp("not a url", None, Duration::from_secs(1)).await;
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid URL"), "msg: {msg}");
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error for invalid URL"),
        }
    }

    #[tokio::test]
    async fn convert_headers_accepts_valid() {
        let mut h = HashMap::new();
        h.insert("X-Token".into(), "abc".into());
        let out = convert_headers(Some(&h)).expect("convert");
        assert_eq!(out.len(), 1);
        assert!(
            out.keys()
                .any(|k| k.as_str().eq_ignore_ascii_case("x-token"))
        );
    }

    #[tokio::test]
    async fn convert_headers_rejects_bad_name() {
        let mut h = HashMap::new();
        h.insert("Bad\nName".into(), "v".into());
        let result = convert_headers(Some(&h));
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid header name"), "msg: {msg}");
            }
            other => panic!("expected Transport err, got {other:?}"),
        }
    }
}
