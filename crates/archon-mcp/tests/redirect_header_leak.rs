//! Regression tests for GHSA-9g45-5xwm-f3wc (custom HTTP headers leak to
//! cross-origin redirect targets), for every archon MCP HTTP transport.
//!
//! reqwest's default redirect policy follows 307/308 and strips only
//! `Authorization`, `Cookie` and `Proxy-Authorization` on a cross-origin hop,
//! so a custom auth header such as `X-Api-Key` is replayed to the redirect
//! target. rmcp 2.1.0 fixed this only in its own `default_http_client()`;
//! archon builds its own clients, so each transport is driven here against an
//! origin that answers with a 307 to a second server. The custom header must
//! reach the configured server and the redirect target must never be hit.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archon_mcp::http_transport::create_http_transport;
use archon_mcp::sse_mcp_transport::connect_mcp;
use archon_mcp::sse_transport::create_sse_transport;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header::LOCATION};
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::{any, get, post};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Notify;

const API_KEY_HEADER: &str = "x-api-key";
const API_KEY_VALUE: &str = "archon-redirect-secret";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Header values one server saw, plus a signal fired on every hit.
#[derive(Clone, Default)]
struct Seen {
    values: Arc<Mutex<Vec<Option<String>>>>,
    hit: Arc<Notify>,
}

impl Seen {
    fn record(&self, headers: &HeaderMap) {
        let value = headers
            .get(API_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        self.values.lock().expect("seen lock").push(value);
        self.hit.notify_one();
    }

    fn values(&self) -> Vec<Option<String>> {
        self.values.lock().expect("seen lock").clone()
    }
}

#[derive(Clone)]
struct OriginState {
    redirect_to: String,
    seen: Seen,
}

async fn redirect(State(state): State<OriginState>, headers: HeaderMap) -> impl IntoResponse {
    state.seen.record(&headers);
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(LOCATION, state.redirect_to)],
        "",
    )
}

/// SSE endpoint that announces `/message` on the same origin and stays open.
async fn sse_with_endpoint(
    State(state): State<OriginState>,
    headers: HeaderMap,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    state.seen.record(&headers);
    let endpoint = Event::default().event("endpoint").data("/message");
    Sse::new(
        futures_util::stream::once(async move { Ok(endpoint) })
            .chain(futures_util::stream::pending()),
    )
}

async fn capture(State(seen): State<Seen>, headers: HeaderMap) -> impl IntoResponse {
    seen.record(&headers);
    (StatusCode::BAD_REQUEST, "redirect target reached")
}

struct Servers {
    origin: SocketAddr,
    origin_seen: Seen,
    target_seen: Seen,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Servers {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn serve(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, task)
}

/// Origin: `POST /mcp`, `GET /sse` and `POST /message` redirect to the
/// target; `GET /sse-ok` is a working SSE stream announcing `/message`.
async fn start_servers() -> Servers {
    let target_seen = Seen::default();
    let (target, target_task) = serve(
        Router::new()
            .route("/capture", any(capture))
            .with_state(target_seen.clone()),
    )
    .await;

    let origin_seen = Seen::default();
    let state = OriginState {
        redirect_to: format!("http://{target}/capture"),
        seen: origin_seen.clone(),
    };
    let (origin, origin_task) = serve(
        Router::new()
            .route("/mcp", post(redirect))
            .route("/sse", get(redirect))
            .route("/message", post(redirect))
            .route("/sse-ok", get(sse_with_endpoint))
            .with_state(state),
    )
    .await;

    Servers {
        origin,
        origin_seen,
        target_seen,
        tasks: vec![target_task, origin_task],
    }
}

fn api_key_headers() -> HashMap<String, String> {
    HashMap::from([(API_KEY_HEADER.to_string(), API_KEY_VALUE.to_string())])
}

fn assert_no_leak(servers: &Servers) {
    let origin = servers.origin_seen.values();
    assert!(
        origin.iter().any(|v| v.as_deref() == Some(API_KEY_VALUE)),
        "the configured server must receive the custom header, saw {origin:?}"
    );
    let target = servers.target_seen.values();
    assert!(
        target.is_empty(),
        "the redirect target must not be contacted at all, saw {target:?}"
    );
}

#[tokio::test]
async fn streamable_http_transport_does_not_follow_redirects() {
    let servers = start_servers().await;
    let transport = create_http_transport(
        &format!("http://{}/mcp", servers.origin),
        Some(&api_key_headers()),
        CONNECT_TIMEOUT,
    )
    .expect("build HTTP transport");

    // The initialize POST hits the redirecting origin, so the handshake cannot
    // succeed; bound it so a hang fails loudly.
    let outcome = tokio::time::timeout(Duration::from_secs(20), rmcp::serve_client((), transport))
        .await
        .expect("initialize must not hang on a redirect");
    assert!(outcome.is_err(), "initialize against a redirect must fail");
    assert_no_leak(&servers);
}

#[tokio::test]
async fn sse_stream_primitive_does_not_follow_redirects() {
    let servers = start_servers().await;
    let transport = create_sse_transport(
        &format!("http://{}/sse", servers.origin),
        Some(&api_key_headers()),
        CONNECT_TIMEOUT,
    )
    .expect("build SSE transport");

    let outcome = transport.connect_sse_stream().await;
    assert!(outcome.is_err(), "an SSE GET answered by a 307 must fail");
    assert_no_leak(&servers);
}

#[tokio::test]
async fn sse_mcp_stream_get_does_not_follow_redirects() {
    let servers = start_servers().await;
    let headers = api_key_headers();
    let url = format!("http://{}/sse", servers.origin);

    // The reconnect pump retries the 307 with backoff and the handshake would
    // wait for its 10 s endpoint timeout; only the first GET matters here.
    let connect = connect_mcp(&url, Some(&headers), CONNECT_TIMEOUT);
    let outcome = tokio::time::timeout(Duration::from_millis(1500), connect).await;
    assert!(
        !matches!(outcome, Ok(Ok(_))),
        "the SSE handshake must not succeed through a redirect"
    );
    assert_no_leak(&servers);
}

#[tokio::test]
async fn sse_mcp_post_does_not_follow_redirects() {
    let servers = start_servers().await;
    let headers = api_key_headers();
    let url = format!("http://{}/sse-ok", servers.origin);
    let (mut sink, _stream) = connect_mcp(&url, Some(&headers), CONNECT_TIMEOUT)
        .await
        .expect("SSE handshake against a working endpoint");

    // GET /sse-ok has been recorded; the POST below is the second origin hit.
    servers.origin_seen.hit.notified().await;
    let ping = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "ping"
    }))
    .expect("json-rpc ping");
    sink.send(ping).await.expect("queue POST");
    tokio::time::timeout(Duration::from_secs(5), servers.origin_seen.hit.notified())
        .await
        .expect("the POST must reach the origin");

    // A followed redirect lands on the target within milliseconds of the 307.
    let leaked =
        tokio::time::timeout(Duration::from_secs(1), servers.target_seen.hit.notified()).await;
    assert!(leaked.is_err(), "the POST redirect target was contacted");
    assert_no_leak(&servers);
    assert_eq!(
        servers.origin_seen.values().len(),
        2,
        "expected exactly the SSE GET and the one POST at the origin"
    );
}
