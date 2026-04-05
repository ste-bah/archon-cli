use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::ws::{Message, WebSocket},
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use serde::Serialize;
use tokio::sync::Mutex;

use super::websocket::{IdeHandlerFn, WsServerConfig};

#[derive(Clone)]
struct ServerState {
    token: String,
    sessions: Arc<Mutex<HashMap<String, ()>>>,
    ide_handler: Option<IdeHandlerFn>,
}

#[derive(Debug, serde::Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    sessions: usize,
}

#[derive(Serialize)]
struct SessionsResponse {
    sessions: Vec<String>,
}

/// An Archon WebSocket server that allows remote agent access.
pub struct WebSocketServer {
    config: WsServerConfig,
    token: String,
}

impl WebSocketServer {
    /// Create a new server. If `config.token` is `None`, a persistent token
    /// is loaded (or generated) from `~/.config/archon/remote-token`.
    pub async fn new(config: WsServerConfig) -> anyhow::Result<Self> {
        let token = match config.token.clone() {
            Some(t) => t,
            None => super::auth::load_or_create_token()?,
        };
        Ok(Self { config, token })
    }

    /// Bind and serve. Blocks until the server is shut down.
    pub async fn run(self) -> anyhow::Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));

        if self.config.tls_cert.is_none() {
            tracing::warn!(
                "remote: WebSocket server starting without TLS on {addr} — \
                 use WSS for remote access"
            );
        }

        println!("Archon WebSocket server listening on ws://{addr}/ws");
        println!(
            "Access token stored at: {}",
            super::auth::token_path().display()
        );

        let state = ServerState {
            token: self.token,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            ide_handler: self.config.ide_handler.clone(),
        };

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/ws", get(ws_handler))
            .route("/ws/ide", get(ws_ide_handler))
            .route("/sessions", get(sessions_handler))
            .route("/sessions/:id", delete(delete_session_handler))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("failed to bind to {addr}: {e}"))?;

        axum::serve(listener, app.into_make_service())
            .await
            .map_err(|e| anyhow::anyhow!("server error: {e}"))
    }
}

async fn health_handler(State(state): State<ServerState>) -> Json<HealthResponse> {
    let sessions = state.sessions.lock().await.len();
    Json(HealthResponse {
        status: "ok",
        sessions,
    })
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<TokenQuery>,
    headers: HeaderMap,
    State(state): State<ServerState>,
) -> Response {
    let provided = extract_bearer_token(&headers, &params.token);

    if !super::auth::validate_token(&state.token, provided.as_deref().unwrap_or("")) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    ws.on_upgrade(move |socket| handle_ws(socket, session_id, state))
}

fn extract_bearer_token(headers: &HeaderMap, query_token: &Option<String>) -> Option<String> {
    if let Some(t) = query_token
        && !t.is_empty()
    {
        return Some(t.clone());
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string())
}

async fn handle_ws(mut socket: WebSocket, session_id: String, state: ServerState) {
    tracing::info!("ws: new session {session_id}");
    state.sessions.lock().await.insert(session_id.clone(), ());

    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                match super::protocol::AgentMessage::from_json_line(text.as_str()) {
                    Ok(agent_msg) => {
                        tracing::debug!("ws: received {:?}", agent_msg);
                        let response = super::protocol::AgentMessage::Event {
                            kind: "echo".to_string(),
                            data: serde_json::json!({"received": true}),
                        };
                        if let Ok(line) = response.to_json_line() {
                            let _ = socket.send(Message::Text(line.into())).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("ws: invalid message from {session_id}: {e}");
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    state.sessions.lock().await.remove(&session_id);
    tracing::info!("ws: session {session_id} closed");
}

/// WebSocket handler for IDE extension connections (`GET /ws/ide`).
///
/// No token auth is required on this endpoint because IDE extensions run
/// locally — they connect via the loopback interface. Each text frame is
/// one complete JSON-RPC 2.0 request (JSON-lines framing); the handler
/// replies with one JSON-RPC response frame.
///
/// The handler is self-contained: it uses `serde_json` directly to avoid
/// a circular crate dependency (archon-core cannot depend on archon-sdk).
async fn ws_ide_handler(ws: WebSocketUpgrade, State(state): State<ServerState>) -> Response {
    ws.on_upgrade(move |socket| handle_ws_ide(socket, state))
}

async fn handle_ws_ide(mut socket: WebSocket, state: ServerState) {
    tracing::info!("ws/ide: IDE client connected");

    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                let response = if let Some(ref handler) = state.ide_handler {
                    // Delegate to the real IdeProtocolHandler injected by the binary layer.
                    handler.lock().await(text.as_str())
                } else {
                    // Fallback stub when no handler is wired (e.g. tests, older integrations).
                    ide_dispatch(text.as_str())
                };
                if let Err(e) = socket.send(Message::Text(response.into())).await {
                    tracing::warn!("ws/ide: failed to send response: {e}");
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    tracing::info!("ws/ide: IDE client disconnected");
}

/// Dispatch one JSON-RPC 2.0 request line and return a JSON-RPC response string.
///
/// This is a thin, stateless dispatcher suitable for the server.rs context.
/// Full session state lives in the archon-sdk IdeProtocolHandler; here we
/// handle method routing for the WebSocket transport layer.
fn ide_dispatch(request_json: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(request_json) {
        Ok(v) => v,
        Err(e) => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":0,"error":{{"code":-32700,"message":"parse error: {e}"}}}}"#
            );
        }
    };

    let id = match v.get("id").and_then(|x| x.as_u64()) {
        Some(id) => id,
        None => {
            return r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32600,"message":"missing id"}}"#
                .to_string();
        }
    };

    let method = match v.get("method").and_then(|x| x.as_str()) {
        Some(m) => m,
        None => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"missing method"}}}}"#
            );
        }
    };

    // Route known methods; delegate full handling to archon-sdk at integration time.
    match method {
        "archon/initialize" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"pending","serverVersion":"{ver}","capabilities":{{"inlineCompletion":false,"toolExecution":false,"diff":false,"terminal":false}}}}}}"#,
            ver = env!("CARGO_PKG_VERSION")
        ),
        "archon/prompt" => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"queued":true}}}}"#),
        "archon/cancel" => {
            format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"cancelled":false}}}}"#)
        }
        "archon/toolResult" => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"ok":true}}}}"#),
        "archon/status" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"model":"","inputTokens":0,"outputTokens":0,"cost":0.0}}}}"#
        ),
        "archon/config" => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"value":null}}}}"#),
        other => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"method not found: {other}"}}}}"#
        ),
    }
}

async fn sessions_handler(State(state): State<ServerState>) -> Json<SessionsResponse> {
    let ids: Vec<String> = state.sessions.lock().await.keys().cloned().collect();
    Json(SessionsResponse { sessions: ids })
}

async fn delete_session_handler(
    Path(id): Path<String>,
    State(state): State<ServerState>,
) -> StatusCode {
    let mut sessions = state.sessions.lock().await;
    if sessions.remove(&id).is_some() {
        tracing::info!("ws: session {id} terminated via API");
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
