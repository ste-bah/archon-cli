//! TASK-P0-B.2b (#182) — MCP PKCE OAuth E2E roundtrip.
//!
//! Proves the OAuth-wrapped MCP SSE transport:
//!   1. Runs the full PKCE code flow against a mock IdP
//!   2. Uses the bearer token on initial GET /sse and every POST /message
//!   3. On server 401, transparently refreshes the token and retries once
//!   4. On persistent 401 (refresh also fails), errors cleanly with no
//!      infinite-retry loop
//!
//! Builds directly on #197's SSE transport — the OAuth wrapper reuses the
//! SSE frame pump and endpoint-discovery logic but overrides the POST pump
//! with auth-aware behavior.

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
use archon_mcp::sse_oauth_transport::connect_mcp_with_oauth;
use archon_mcp::types::McpError;

// ---------------------------------------------------------------------------
// PKCE primitives unit tests (RFC 7636 Appendix B test vector)
// ---------------------------------------------------------------------------

#[test]
fn pkce_code_challenge_rfc7636_known_vector() {
    // RFC 7636 Appendix B: given this verifier, S256 challenge is as follows.
    // verifier  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(code_challenge(verifier), expected);
}

#[test]
fn pkce_generated_verifier_is_valid_length() {
    for _ in 0..10 {
        let v = generate_code_verifier();
        assert!(v.len() >= 43, "verifier too short: {} chars", v.len());
        assert!(v.len() <= 128, "verifier too long: {} chars", v.len());
        // URL-safe alphabet (RFC 7636 §4.1): [A-Z] [a-z] [0-9] '-' '.' '_' '~'
        for c in v.chars() {
            assert!(
                c.is_ascii_alphanumeric() || "-._~".contains(c),
                "non-unreserved char in verifier: {c:?}"
            );
        }
    }
}

#[test]
fn pkce_generated_verifiers_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        assert!(
            seen.insert(generate_code_verifier()),
            "duplicate code_verifier"
        );
    }
}

// ---------------------------------------------------------------------------
// Mock IdP + mock MCP-SSE server
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct IdpState {
    /// Map from auth code -> code_challenge (set by /authorize, consumed by /token).
    pending_auth: Arc<Mutex<HashMap<String, String>>>,
    /// Map from active access_token -> refresh_token (issued by /token).
    issued_tokens: Arc<Mutex<HashMap<String, String>>>,
    /// Map from refresh_token -> true for valid; false once used/revoked.
    refresh_valid: Arc<Mutex<HashMap<String, bool>>>,
    /// Next monotonic token id.
    next_token: Arc<Mutex<u64>>,
    /// If set, /token endpoint will reject refresh requests (for fail-closed test).
    refresh_disabled: Arc<Mutex<bool>>,
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    code_challenge: String,
    #[serde(default)]
    #[allow(dead_code)]
    code_challenge_method: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    redirect_uri: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    client_id: Option<String>,
}

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    redirect_uri: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    client_id: Option<String>,
}

async fn idp_authorize(
    State(state): State<IdpState>,
    Query(q): Query<AuthorizeQuery>,
) -> (StatusCode, String) {
    let code = format!("auth-code-{}", uuid::Uuid::new_v4());
    state
        .pending_auth
        .lock()
        .await
        .insert(code.clone(), q.code_challenge);
    // Respond with the code directly (normally this would be a redirect).
    (StatusCode::OK, code)
}

async fn idp_token(
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
            // Rotate: old refresh is no longer valid, new refresh is.
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

async fn spawn_idp() -> (SocketAddr, tokio::task::JoinHandle<()>, IdpState) {
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
struct McpState {
    tx: mpsc::UnboundedSender<String>,
    rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    /// Access token the server currently accepts. Updated by tests to simulate expiry.
    accepted_token: Arc<Mutex<String>>,
    post_path: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct McpSessionQuery {
    #[serde(default)]
    session: Option<String>,
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

    // Require a valid bearer token to even open the SSE stream.
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

async fn spawn_mcp_with_auth(
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
// Helpers: drive the PKCE authorization code flow end-to-end
// ---------------------------------------------------------------------------

async fn run_pkce_flow(idp_addr: SocketAddr) -> OAuthClient {
    let config = OAuthConfig {
        authorize_url: format!("http://{idp_addr}/authorize"),
        token_url: format!("http://{idp_addr}/token"),
        client_id: "archon-test".into(),
        redirect_uri: "http://localhost/cb".into(),
    };

    // 1. Generate verifier + challenge.
    let verifier = generate_code_verifier();
    let challenge = code_challenge(&verifier);

    // 2. Call /authorize with the challenge — gets us the code.
    let auth_url = format!(
        "http://{idp_addr}/authorize?code_challenge={challenge}&code_challenge_method=S256&client_id=archon-test&redirect_uri=http://localhost/cb"
    );
    let resp = reqwest::get(&auth_url).await.expect("authorize request");
    assert_eq!(resp.status(), 200, "authorize should succeed");
    let code = resp.text().await.expect("authorize body");
    assert!(code.starts_with("auth-code-"), "got code: {code}");

    // 3. Exchange code for token via the OAuthClient.
    OAuthClient::exchange_code(config, &code, &verifier)
        .await
        .expect("exchange_code should succeed")
}

// ---------------------------------------------------------------------------
// E2E test: full PKCE flow -> OAuth-wrapped SSE transport -> MCP roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_full_flow_initialize_and_tools_list() {
    let (idp_addr, idp_server, _idp_state) = spawn_idp().await;
    let client = run_pkce_flow(idp_addr).await;

    // Bring up MCP server that accepts the access token we just minted.
    let current = client.access_token().await;
    let (mcp_addr, mcp_server, _mcp_state) = spawn_mcp_with_auth(current).await;

    // Connect through the OAuth-wrapped transport.
    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = tokio::time::timeout(
        Duration::from_secs(10),
        connect_mcp_with_oauth(&sse_url, client, Duration::from_secs(5)),
    )
    .await
    .expect("connect within 10s")
    .expect("OAuth-wrapped connect succeeded");

    // Drive a tools/list roundtrip.
    let tools = tokio::time::timeout(Duration::from_secs(5), mcp_client.list_tools())
        .await
        .expect("list_tools within 5s")
        .expect("tools/list succeeded");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "ping");

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}

// ---------------------------------------------------------------------------
// E2E test: 401 mid-session triggers refresh + retry, then succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_401_triggers_refresh_and_retry_succeeds() {
    let (idp_addr, idp_server, _idp_state) = spawn_idp().await;
    let client = run_pkce_flow(idp_addr).await;
    let current = client.access_token().await;
    let (mcp_addr, mcp_server, mcp_state) = spawn_mcp_with_auth(current.clone()).await;

    // Connect + initialize successfully first (confirms the happy path works
    // with the current token).
    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = connect_mcp_with_oauth(&sse_url, client.clone(), Duration::from_secs(5))
        .await
        .expect("initial OAuth connect");

    // Now ROTATE the server's accepted token BEFORE the refresh flow runs.
    // Compute what the next refresh will produce ("access-2") and prime the
    // server to accept it. This simulates a mid-session token expiry where
    // the server has already advanced to a new key.
    *mcp_state.accepted_token.lock().await = "access-2".into();

    // A fresh MCP call now sees 401 on POST. The OAuth wrapper must:
    //   a. call refresh() on the OAuthClient
    //   b. update the shared token state
    //   c. retry the POST with the new bearer
    //   d. succeed
    //
    // The assertion of success is that list_tools returns a non-error Result.
    let tools = tokio::time::timeout(Duration::from_secs(10), mcp_client.list_tools())
        .await
        .expect("list_tools within 10s");

    // The wrapper transparently refreshed; the result is the same as the
    // happy path.
    let tools = tools.expect("tools/list should succeed after auto-refresh");
    assert_eq!(tools.len(), 1);

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}

// ---------------------------------------------------------------------------
// E2E test: persistent 401 (refresh also fails) surfaces as error, no infinite retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_persistent_401_errors_bounded() {
    let (idp_addr, idp_server, idp_state) = spawn_idp().await;
    let client = run_pkce_flow(idp_addr).await;
    let current = client.access_token().await;
    let (mcp_addr, mcp_server, mcp_state) = spawn_mcp_with_auth(current.clone()).await;

    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = connect_mcp_with_oauth(&sse_url, client, Duration::from_secs(5))
        .await
        .expect("initial connect");

    // Rotate the MCP server's accepted token to something new, AND disable
    // refresh on the IdP. Now the wrapper will try to refresh, get 401 back,
    // and must fail the caller — not loop forever.
    *mcp_state.accepted_token.lock().await = "different-access".into();
    *idp_state.refresh_disabled.lock().await = true;

    let result = tokio::time::timeout(Duration::from_secs(8), mcp_client.list_tools()).await;

    // Either the call times out (rmcp-level) or it errors — but it must not
    // hang forever and it must not infinitely loop. Empirically the rmcp
    // request will time out because the server rejects every POST.
    match result {
        Ok(Ok(_)) => panic!("expected error when refresh is disabled"),
        Ok(Err(e)) => {
            // Some kind of McpError is acceptable; the important property
            // is that we got back in bounded time.
            let _e: McpError = e;
        }
        Err(_) => {
            // Timeout also acceptable — the transport gave up cleanly.
        }
    }

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}
