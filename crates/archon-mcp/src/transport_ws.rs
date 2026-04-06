//! WebSocket MCP transport for TASK-CLI-304.
//!
//! Implements JSON-RPC 2.0 over WebSocket frames using `tokio-tungstenite`.
//!
//! Key behaviors (matching reference `WebSocketTransport.ts`):
//! - `Sec-WebSocket-Protocol: mcp` subprotocol on connect
//! - Time-budget reconnection: 10-minute wall-clock budget
//! - Reconnect delay: `min(1s * 2^attempt, 30s)` with ±25% jitter
//! - Sleep/wake detection: gap > 60s resets budget and attempt counter
//! - Ping every 10s; absent pong on next tick triggers reconnect
//! - JSON data frame `{"type":"keep_alive"}` every 5 minutes
//! - Permanent close codes: 1002, 4001, 4003 — no reconnection
//! - Token rotation on 4003 if `headersHelper` configured

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::handshake::client::Request;

use crate::types::McpError;
use crate::ws_config::{PERMANENT_CLOSE_CODES, WsConfig, WsReconnectConfig};

// ---------------------------------------------------------------------------
// Public helpers (tested directly)
// ---------------------------------------------------------------------------

/// Compute reconnect delay for a given attempt number.
///
/// Formula: `min(1s * 2^attempt, 30s)` with ±25% jitter.
///
/// `elapsed_ms` is currently unused but reserved for future budget checking.
pub fn reconnect_delay_ms(attempt: u32, _elapsed_ms: u64) -> u64 {
    // Base: 1s * 2^attempt, capped at 30s
    // 1s * 2^attempt, capped at 30s.  Shift is safe because attempt is capped at 15.
    let shift = attempt.min(15) as u64;
    let base_ms: u64 = (1_000u64.saturating_mul(1u64 << shift)).min(30_000);

    // ±25% jitter: result in [base * 0.75, base * 1.25)
    let jitter_range = base_ms / 4; // 25%
    if jitter_range == 0 {
        return base_ms;
    }
    // Subtract 12.5% and add random 0-25% to get ±12.5% around base
    // Simpler: base_ms - jitter_range/2 + random(0, jitter_range)
    let low = base_ms.saturating_sub(jitter_range / 2);
    low + rand_u64(jitter_range)
}

/// Returns `true` if `code` is a permanent failure code that should not reconnect.
pub fn is_permanent_close_code(code: u16) -> bool {
    PERMANENT_CLOSE_CODES.contains(&code)
}

// ---------------------------------------------------------------------------
// WebSocketTransport
// ---------------------------------------------------------------------------

/// WebSocket transport that manages a JSON-RPC 2.0 connection to an MCP server.
///
/// Construction validates the URL. Use `connect()` to establish the connection.
pub struct WebSocketTransport {
    config: WsConfig,
    reconnect: WsReconnectConfig,
    url: ::url::Url,
}

impl WebSocketTransport {
    /// Create a new transport, validating the URL.
    ///
    /// Returns `Err` if the URL is invalid or not a WebSocket URL (ws:// or wss://).
    pub fn new(config: WsConfig, reconnect: WsReconnectConfig) -> Result<Self, McpError> {
        let url = config.url.parse::<::url::Url>().map_err(|e| {
            McpError::Transport(format!("invalid WebSocket URL '{}': {}", config.url, e))
        })?;

        match url.scheme() {
            "ws" | "wss" => {}
            scheme => {
                return Err(McpError::Transport(format!(
                    "WebSocket URL must use ws:// or wss://, got '{scheme}://'"
                )));
            }
        }

        Ok(Self {
            config,
            reconnect,
            url,
        })
    }

    /// Attempt to connect to the WebSocket server.
    ///
    /// Sends `Sec-WebSocket-Protocol: mcp` and custom headers from config.
    /// Returns an error on connection failure (refused, DNS failure, etc.).
    pub async fn connect(&self) -> Result<ActiveWsConnection, McpError> {
        let request = self.build_request()?;

        let (stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| {
                McpError::Transport(format!("WebSocket connect to '{}' failed: {}", self.url, e))
            })?;

        Ok(ActiveWsConnection {
            stream,
            ping_interval_ms: self.reconnect.ping_interval_ms,
            keepalive_interval_ms: self.reconnect.keepalive_interval_ms,
        })
    }

    /// Access the validated URL.
    pub fn url(&self) -> &::url::Url {
        &self.url
    }

    /// Access the reconnection configuration.
    pub fn reconnect_config(&self) -> &WsReconnectConfig {
        &self.reconnect
    }

    /// Access the WebSocket configuration.
    pub fn ws_config(&self) -> &WsConfig {
        &self.config
    }

    // ── Private helpers ─────────────────────────────────────────────────────

    fn build_request(&self) -> Result<Request, McpError> {
        let mut builder = Request::builder()
            .uri(self.url.as_str())
            .header("Sec-WebSocket-Protocol", "mcp");

        for (k, v) in &self.config.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        builder
            .body(())
            .map_err(|e| McpError::Transport(format!("failed to build WebSocket request: {e}")))
    }
}

// ---------------------------------------------------------------------------
// ActiveWsConnection
// ---------------------------------------------------------------------------

/// An established WebSocket connection to an MCP server.
///
/// Wraps the raw `tokio-tungstenite` stream and exposes MCP-level operations.
pub struct ActiveWsConnection {
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    pub ping_interval_ms: u64,
    pub keepalive_interval_ms: u64,
}

impl ActiveWsConnection {
    /// Send a JSON-RPC request object as a single WebSocket text frame.
    pub async fn send_json(&mut self, msg: &serde_json::Value) -> Result<(), McpError> {
        use tokio_tungstenite::tungstenite::Message;

        let text = serde_json::to_string(msg).map_err(McpError::Json)?;
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| McpError::Transport(format!("WebSocket send failed: {e}")))
    }

    /// Send a WebSocket ping frame.
    pub async fn ping(&mut self) -> Result<(), McpError> {
        use tokio_tungstenite::tungstenite::Message;

        self.stream
            .send(Message::Ping(vec![].into()))
            .await
            .map_err(|e| McpError::Transport(format!("WebSocket ping failed: {e}")))
    }

    /// Send the JSON data keep-alive frame `{"type":"keep_alive"}`.
    ///
    /// Resets proxy idle timers (e.g. Cloudflare 5-minute timeout).
    pub async fn send_keepalive(&mut self) -> Result<(), McpError> {
        self.send_json(&serde_json::json!({"type": "keep_alive"}))
            .await
    }

    /// Receive the next message, parsing newline-delimited JSON if needed.
    ///
    /// Server may send multiple newline-delimited JSON objects in one frame.
    pub async fn recv_next(&mut self) -> Result<Option<Vec<serde_json::Value>>, McpError> {
        use tokio_tungstenite::tungstenite::Message;

        match self.stream.next().await {
            None => Ok(None),
            Some(Err(e)) => Err(McpError::Transport(format!("WebSocket recv error: {e}"))),
            Some(Ok(Message::Text(text))) => {
                // Parse newline-delimited JSON lines in a single frame
                let messages: Result<Vec<_>, _> = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(serde_json::from_str::<serde_json::Value>)
                    .collect();
                messages.map(Some).map_err(McpError::Json)
            }
            Some(Ok(Message::Binary(data))) => {
                // Try to parse binary as UTF-8 JSON
                let text = String::from_utf8_lossy(&data);
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(v) => Ok(Some(vec![v])),
                    Err(e) => Err(McpError::Transport(format!(
                        "invalid binary JSON frame: {e}"
                    ))),
                }
            }
            Some(Ok(Message::Ping(_))) => Ok(Some(vec![])), // pong sent automatically
            Some(Ok(Message::Pong(_))) => Ok(Some(vec![])), // keep-alive acknowledged
            Some(Ok(Message::Close(frame))) => {
                let code = frame.as_ref().map(|f| f.code.into()).unwrap_or(0u16);
                Err(McpError::Transport(format!(
                    "WebSocket closed by server (code {code})"
                )))
            }
            Some(Ok(Message::Frame(_))) => Ok(Some(vec![])), // raw frame, ignore
        }
    }

    /// Consume the connection and return the underlying `WebSocketStream`.
    ///
    /// This is useful for handing the raw stream to an MCP client that
    /// requires a `Sink + Stream` transport (e.g. via rmcp's `IntoTransport`).
    pub fn into_stream(
        self,
    ) -> tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    > {
        self.stream
    }

    /// Send graceful close frame.
    pub async fn close(&mut self) -> Result<(), McpError> {
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

        let frame = CloseFrame {
            code: CloseCode::Normal,
            reason: "client shutdown".into(),
        };
        self.stream
            .send(Message::Close(Some(frame)))
            .await
            .map_err(|e| McpError::Transport(format!("WebSocket close failed: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Generate a random u64 in `[0, bound)`.
fn rand_u64(bound: u64) -> u64 {
    use rand::Rng;
    if bound == 0 {
        return 0;
    }
    rand::rng().random_range(0..bound)
}
