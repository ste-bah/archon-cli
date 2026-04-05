use std::path::PathBuf;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{RemoteSession, RemoteSessionInner, protocol::AgentMessage};

/// Configuration for a WebSocket client connection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WsConnectionConfig {
    /// WebSocket URL to connect to (e.g. `ws://localhost:8420/ws`).
    pub url: String,
    /// Bearer token sent in the `Authorization` header.
    pub token: String,
    /// Whether to attempt reconnection on disconnect.
    pub reconnect: bool,
    /// Maximum number of reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Session identifier (auto-generated if empty).
    pub session_id: String,
}

impl Default for WsConnectionConfig {
    fn default() -> Self {
        Self {
            url: "ws://localhost:8420/ws".to_string(),
            token: String::new(),
            reconnect: true,
            max_reconnect_attempts: 5,
            session_id: String::new(),
        }
    }
}

/// A callable that handles one JSON-RPC request string and returns a response string.
/// Used to inject `archon-sdk::ide::handler::IdeProtocolHandler` from the binary layer,
/// avoiding a circular crate dependency (archon-core ← archon-sdk ← archon-core).
pub type IdeHandlerFn = std::sync::Arc<tokio::sync::Mutex<Box<dyn FnMut(&str) -> String + Send>>>;

/// Configuration for the WebSocket server.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WsServerConfig {
    /// Port to listen on.
    pub port: u16,
    /// Path to TLS certificate (PEM). If `None`, server runs without TLS.
    pub tls_cert: Option<PathBuf>,
    /// Path to TLS private key (PEM). Required when `tls_cert` is set.
    pub tls_key: Option<PathBuf>,
    /// Override bearer token. If `None`, `load_or_create_token()` is used.
    pub token: Option<String>,
    /// Maximum number of concurrent WebSocket sessions.
    pub max_sessions: u32,
    /// Optional IDE JSON-RPC handler injected by the binary.
    /// When `Some`, the `/ws/ide` endpoint delegates to this handler instead of the stub.
    #[serde(skip)]
    pub ide_handler: Option<IdeHandlerFn>,
}

impl std::fmt::Debug for WsServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsServerConfig")
            .field("port", &self.port)
            .field("tls_cert", &self.tls_cert)
            .field("tls_key", &self.tls_key)
            .field("token", &self.token)
            .field("max_sessions", &self.max_sessions)
            .field("ide_handler", &self.ide_handler.is_some())
            .finish()
    }
}

impl Default for WsServerConfig {
    fn default() -> Self {
        Self {
            port: 8420,
            tls_cert: None,
            tls_key: None,
            token: None,
            max_sessions: 16,
            ide_handler: None,
        }
    }
}

/// WebSocket transport — establishes outbound connections to a remote server.
pub struct WsTransport;

impl WsTransport {
    /// Connect to a WebSocket server at `config.url`, authenticating with
    /// `config.token` via the `Authorization: Bearer` header.
    pub async fn connect_ws(&self, config: &WsConnectionConfig) -> anyhow::Result<RemoteSession> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::header;

        let mut request = config
            .url
            .as_str()
            .into_client_request()
            .map_err(|e| anyhow::anyhow!("invalid WebSocket URL '{}': {e}", config.url))?;

        if !config.token.is_empty() {
            let header_value = format!("Bearer {}", config.token)
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid token for Authorization header: {e}"))?;
            request
                .headers_mut()
                .insert(header::AUTHORIZATION, header_value);
        }

        let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| anyhow::anyhow!("WebSocket connection to '{}' failed: {e}", config.url))?;

        let session_id = if config.session_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            config.session_id.clone()
        };

        tracing::info!(
            "ws: connected to '{}' session_id={}",
            config.url,
            session_id
        );

        let inner = WsSessionInner {
            stream: Mutex::new(ws_stream),
        };

        Ok(RemoteSession {
            session_id,
            inner: Box::new(inner),
        })
    }
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// The stream field is accessed via the Mutex inside trait method impls.
#[allow(dead_code)]
struct WsSessionInner {
    stream: Mutex<WsStream>,
}

#[async_trait]
impl RemoteSessionInner for WsSessionInner {
    async fn send(&self, message: &AgentMessage) -> anyhow::Result<()> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let line = message.to_json_line()?;
        let mut stream = self.stream.lock().await;
        stream
            .send(Message::Text(line.into()))
            .await
            .map_err(|e| anyhow::anyhow!("ws: send failed: {e}"))
    }

    async fn recv(&self) -> anyhow::Result<AgentMessage> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        loop {
            let msg = {
                let mut stream = self.stream.lock().await;
                stream.next().await
            };
            match msg {
                Some(Ok(Message::Text(text))) => {
                    return AgentMessage::from_json_line(text.as_str());
                }
                Some(Ok(Message::Close(_))) | None => {
                    anyhow::bail!("ws: connection closed");
                }
                Some(Ok(
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_),
                )) => {
                    // Ignore control frames and binary messages.
                }
                Some(Err(e)) => {
                    anyhow::bail!("ws: receive error: {e}");
                }
            }
        }
    }

    async fn disconnect(&self) -> anyhow::Result<()> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let mut stream = self.stream.lock().await;
        stream
            .send(Message::Close(None))
            .await
            .map_err(|e| anyhow::anyhow!("ws: close failed: {e}"))
    }
}
