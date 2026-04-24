//! Shared helpers for the `oauth_pkce_*` integration test binaries.
//!
//! Extracted from the original `tests/oauth_pkce_roundtrip.rs` as part of
//! #204 HYGIENE-MCP-FILE-SIZES split. Each per-scenario test binary pulls
//! this module in via `mod common;` — cargo treats `tests/common/` as a
//! subdirectory (not a separate test binary) so the shared code is compiled
//! once per consumer.
//!
//! What lives here:
//!   * Mock IdP (axum): `/authorize` + `/token` with PKCE S256 verification
//!     and refresh-token rotation. Exposed via `spawn_idp`.
//!   * Mock MCP-SSE server with bearer-token gating on both GET `/sse`
//!     and POST `/message`. Exposed via `spawn_mcp_with_auth`.
//!   * `run_pkce_flow` — drives verifier/challenge → /authorize → /token
//!     and returns a live `OAuthClient`.

#![allow(dead_code)]
// Each consuming test binary only uses a subset of these helpers.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};

use archon_mcp::oauth_pkce::{OAuthClient, OAuthConfig, code_challenge, generate_code_verifier};

// ---------------------------------------------------------------------------
// Mock IdP state + handlers
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct IdpState {
    /// Map from auth code -> code_challenge (set by /authorize, consumed by /token).
    pub pending_auth: Arc<Mutex<HashMap<String, String>>>,
    /// Map from active access_token -> refresh_token (issued by /token).
    pub issued_tokens: Arc<Mutex<HashMap<String, String>>>,
    /// Map from refresh_token -> true for valid; false once used/revoked.
    pub refresh_valid: Arc<Mutex<HashMap<String, bool>>>,
    /// Next monotonic token id.
    pub next_token: Arc<Mutex<u64>>,
    /// If set, /token endpoint rejects refresh requests (for fail-closed test).
    pub refresh_disabled: Arc<Mutex<bool>>,
}

#[derive(Deserialize)]
pub struct AuthorizeQuery {
    pub code_challenge: String,
    #[serde(default)]
    pub code_challenge_method: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

pub async fn idp_authorize(
    State(state): State<IdpState>,
    Query(q): Query<AuthorizeQuery>,
) -> (StatusCode, String) {
    let code = format!("auth-code-{}", uuid::Uuid::new_v4());
    state
        .pending_auth
        .lock()
        .await
        .insert(code.clone(), q.code_challenge);
    (StatusCode::OK, code)
}

pub async fn idp_token(
    State(state): State<IdpState>,
    Form(body): Form<TokenForm>,
) -> (StatusCode, Json<serde_json::Value>) {
    match body.grant_type.as_str() {
        "authorization_code" => {
            let code = match body.code {
                Some(c) => c,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": "missing code" })),
                    );
                }
            };
            let verifier = match body.code_verifier {
                Some(v) => v,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": "missing code_verifier" })),
                    );
                }
            };
            let stored_challenge = state.pending_auth.lock().await.remove(&code);
            let stored_challenge = match stored_challenge {
                Some(c) => c,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": "unknown code" })),
                    );
                }
            };
            let computed = code_challenge(&verifier);
            if computed != stored_challenge {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "code_verifier mismatch" })),
                );
            }

            let mut n = state.next_token.lock().await;
            *n += 1;
            let id = *n;
            drop(n);
            let access = format!("access-{id}");
            let refresh = format!("refresh-{id}");
            state
                .issued_tokens
                .lock()
                .await
                .insert(access.clone(), refresh.clone());
            state
                .refresh_valid
                .lock()
                .await
                .insert(refresh.clone(), true);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "access_token": access,
                    "refresh_token": refresh,
                    "token_type": "Bearer",
                    "expires_in": 3600
                })),
            )
        }
        "refresh_token" => {
            if *state.refresh_disabled.lock().await {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({ "error": "refresh disabled" })),
                );
            }
            let rt = match body.refresh_token {
                Some(r) => r,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": "missing refresh_token" })),
                    );
                }
            };
            let valid = state
                .refresh_valid
                .lock()
                .await
                .get(&rt)
                .copied()
                .unwrap_or(false);
            if !valid {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({ "error": "invalid refresh_token" })),
                );
            }

            let mut n = state.next_token.lock().await;
            *n += 1;
            let id = *n;
            drop(n);
            let access = format!("access-{id}");
            let refresh = format!("refresh-{id}");
            state.refresh_valid.lock().await.insert(rt, false);
            state
                .refresh_valid
                .lock()
                .await
                .insert(refresh.clone(), true);
            state
                .issued_tokens
                .lock()
                .await
                .insert(access.clone(), refresh.clone());
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "access_token": access,
                    "refresh_token": refresh,
                    "token_type": "Bearer",
                    "expires_in": 3600
                })),
            )
        }
        other => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unsupported grant_type '{other}'") })),
        ),
    }
}

pub async fn spawn_idp() -> (SocketAddr, tokio::task::JoinHandle<()>, IdpState) {
    let state = IdpState::default();
    let app = Router::new()
        .route("/authorize", get(idp_authorize))
        .route("/token", post(idp_token))
        .with_state(state.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server, state)
}

// ---------------------------------------------------------------------------
// Mock MCP-SSE server with bearer-token gating
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct McpState {
    pub tx: mpsc::UnboundedSender<String>,
    pub rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    /// Access token the server currently accepts. Tests rotate this to
    /// simulate token expiry.
    pub accepted_token: Arc<Mutex<String>>,
    pub post_path: String,
}

#[derive(Deserialize)]
pub struct McpSessionQuery {
    #[serde(default)]
    pub session: Option<String>,
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

async fn mcp_sse(
    State(state): State<McpState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    use futures_util::StreamExt;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    let bearer = extract_bearer(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let expected = state.accepted_token.lock().await.clone();
    if bearer != expected {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let rx = state
        .rx
        .lock()
        .await
        .take()
        .expect("mcp_sse called twice");
    let endpoint_data = state.post_path.clone();

    let endpoint_frame = futures_util::stream::once(async move {
        Ok(Event::default().event("endpoint").data(endpoint_data))
    });
    let responses = UnboundedReceiverStream::new(rx)
        .map(|body| Ok::<Event, Infallible>(Event::default().event("message").data(body)));

    Ok(Sse::new(endpoint_frame.chain(responses))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(30))))
}

async fn mcp_message(
    State(state): State<McpState>,
    Query(_q): Query<McpSessionQuery>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> StatusCode {
    let bearer = match extract_bearer(&headers) {
        Some(b) => b,
        None => return StatusCode::UNAUTHORIZED,
    };
    let expected = state.accepted_token.lock().await.clone();
    if bearer != expected {
        return StatusCode::UNAUTHORIZED;
    }

    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED;
    };
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let resp = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "oauth-mock", "version": "0.1.0" }
            }
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": [{
                "name": "ping",
                "description": "Ping the server",
                "inputSchema": { "type": "object", "properties": {} }
            }]}
        }),
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("unsupported method '{other}' in oauth mock")
            }
        }),
    };
    let _ = state.tx.send(serde_json::to_string(&resp).unwrap());
    StatusCode::ACCEPTED
}

pub async fn spawn_mcp_with_auth(
    accepted_token: String,
) -> (SocketAddr, tokio::task::JoinHandle<()>, McpState) {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let state = McpState {
        tx,
        rx: Arc::new(Mutex::new(Some(rx))),
        accepted_token: Arc::new(Mutex::new(accepted_token)),
        post_path: "/message?session=oauth-test".into(),
    };
    let app = Router::new()
        .route("/sse", get(mcp_sse))
        .route("/message", post(mcp_message))
        .with_state(state.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server, state)
}

// ---------------------------------------------------------------------------
// PKCE flow driver
// ---------------------------------------------------------------------------

/// Drive the full PKCE authorization-code flow against a mock IdP and return
/// a live `OAuthClient` holding the issued token pair.
pub async fn run_pkce_flow(idp_addr: SocketAddr) -> OAuthClient {
    let config = OAuthConfig {
        authorize_url: format!("http://{idp_addr}/authorize"),
        token_url: format!("http://{idp_addr}/token"),
        client_id: "archon-test".into(),
        redirect_uri: "http://localhost/cb".into(),
    };

    let verifier = generate_code_verifier();
    let challenge = code_challenge(&verifier);

    let auth_url = format!(
        "http://{idp_addr}/authorize?code_challenge={challenge}&code_challenge_method=S256&client_id=archon-test&redirect_uri=http://localhost/cb"
    );
    let resp = reqwest::get(&auth_url).await.expect("authorize request");
    assert_eq!(resp.status(), 200, "authorize should succeed");
    let code = resp.text().await.expect("authorize body");
    assert!(code.starts_with("auth-code-"), "got code: {code}");

    OAuthClient::exchange_code(config, &code, &verifier)
        .await
        .expect("exchange_code should succeed")
}
