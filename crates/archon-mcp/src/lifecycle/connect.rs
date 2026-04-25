//! Per-transport connection dispatch for the MCP server manager.
//!
//! `connect_server` is the single entry point that builds an appropriate
//! transport for a `ServerConfig` (stdio / http / ws / sse) and runs the
//! MCP client initialization handshake on it. The internal-only WebSocket
//! JSON-RPC adapter `create_ws_json_rpc_transport` lives here too.
//!
//! `connect_server_for_test` is a `#[doc(hidden)] pub` wrapper that exposes
//! `connect_server` to integration tests under `tests/`.

use crate::client::McpClient;
use crate::http_transport::create_http_transport;
use crate::sse_mcp_transport::connect_mcp as connect_sse_mcp;
use crate::transport::spawn_transport;
use crate::transport_ws::WebSocketTransport;
use crate::types::{McpError, ServerConfig};
use crate::ws_config::{WsConfig, WsReconnectConfig};

use super::HTTP_CONNECT_TIMEOUT;

/// Create an appropriate transport and initialize an MCP client for a server config.
///
/// Dispatches on `config.transport`:
///   * `"stdio"` or `""` — child-process JSON-RPC over stdio
///   * `"http"`          — bidirectional HTTP streaming transport
///   * `"ws"` / `"websocket"` — WebSocket JSON-RPC
///   * `"sse"`           — classic MCP Server-Sent Events (GET /sse + POST /message)
pub(super) async fn connect_server(config: &ServerConfig) -> Result<McpClient, McpError> {
    match config.transport.as_str() {
        "http" => {
            let url = config.url.as_deref().ok_or_else(|| {
                McpError::Transport(format!(
                    "server '{}' has transport=http but no url configured",
                    config.name
                ))
            })?;
            let transport =
                create_http_transport(url, config.headers.as_ref(), HTTP_CONNECT_TIMEOUT)?;
            McpClient::initialize(config, transport).await
        }
        "stdio" | "" => {
            let transport = spawn_transport(config)?;
            McpClient::initialize(config, transport).await
        }
        "ws" | "websocket" => {
            let url = config.url.as_deref().ok_or_else(|| {
                McpError::Transport(format!(
                    "server '{}' has transport=ws but no url configured",
                    config.name
                ))
            })?;
            let ws_config = WsConfig {
                url: url.to_string(),
                headers: config.headers.clone().unwrap_or_default(),
                headers_helper: None,
            };
            let ws_transport = WebSocketTransport::new(ws_config, WsReconnectConfig::default())?;
            let active = ws_transport.connect().await?;
            let ws_stream = active.into_stream();
            let transport = create_ws_json_rpc_transport(ws_stream);
            McpClient::initialize(config, transport).await
        }
        "sse" => {
            let url = config.url.as_deref().ok_or_else(|| {
                McpError::Transport(format!(
                    "server '{}' has transport=sse but no url configured",
                    config.name
                ))
            })?;
            let transport =
                connect_sse_mcp(url, config.headers.as_ref(), HTTP_CONNECT_TIMEOUT).await?;
            McpClient::initialize(config, transport).await
        }
        other => {
            tracing::warn!(
                server = %config.name,
                transport = %other,
                "unknown transport type, skipping server"
            );
            Err(McpError::Transport(format!(
                "unknown transport type '{}' for server '{}'",
                other, config.name
            )))
        }
    }
}

/// Wrap a raw `WebSocketStream` into a `(Sink, Stream)` pair that speaks
/// `JsonRpcMessage`, suitable for passing to `McpClient::initialize()`.
///
/// - **Outgoing**: serializes `JsonRpcMessage` to JSON and sends as a text frame.
/// - **Incoming**: deserializes text frames into `JsonRpcMessage`, ignoring
///   non-text frames (ping/pong/binary/close).
fn create_ws_json_rpc_transport(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> (
    impl futures_util::Sink<
        rmcp::service::TxJsonRpcMessage<rmcp::service::RoleClient>,
        Error = tokio_tungstenite::tungstenite::Error,
    > + Send
    + Unpin
    + 'static,
    impl futures_util::Stream<Item = rmcp::service::RxJsonRpcMessage<rmcp::service::RoleClient>>
    + Send
    + Unpin
    + 'static,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (sink, stream) = ws.split();

    // Map the sink: JsonRpcMessage -> serialize -> Message::Text
    // Uses `with` + sync closure to keep Unpin.
    let mapped_sink = sink.with(
        |msg: rmcp::service::TxJsonRpcMessage<rmcp::service::RoleClient>| {
            let result = serde_json::to_string(&msg)
                .map(|json| Message::Text(json.into()))
                .map_err(|e| {
                    tokio_tungstenite::tungstenite::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("ws json serialize: {e}"),
                    ))
                });
            futures_util::future::ready(result)
        },
    );

    // Map the stream: Message -> extract text -> deserialize -> JsonRpcMessage
    let mapped_stream = stream.filter_map(|result| {
        futures_util::future::ready(match result {
            Ok(Message::Text(text)) => {
                match serde_json::from_str::<
                    rmcp::service::RxJsonRpcMessage<rmcp::service::RoleClient>,
                >(&text)
                {
                    Ok(msg) => Some(msg),
                    Err(e) => {
                        tracing::warn!("ws: failed to parse JSON-RPC message: {e}");
                        None
                    }
                }
            }
            Ok(_) => None, // ignore ping/pong/binary/close
            Err(e) => {
                tracing::warn!("ws: stream error: {e}");
                None
            }
        })
    });

    (mapped_sink, mapped_stream)
}

/// Test-only helper: expose `connect_server` to integration tests under
/// `tests/`. The private `connect_server` stays private for production code.
///
/// Used by `tests/sse_transport_roundtrip.rs` to verify the classic-SSE
/// transport match arm end-to-end without duplicating the entire
/// dispatch logic inside each test file.
#[doc(hidden)]
pub async fn connect_server_for_test(config: &ServerConfig) -> Result<McpClient, McpError> {
    connect_server(config).await
}
