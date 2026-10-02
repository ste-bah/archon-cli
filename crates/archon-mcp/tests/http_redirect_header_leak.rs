//! Regression test for GHSA-9g45-5xwm-f3wc (custom HTTP headers leak to
//! cross-origin redirect targets).
//!
//! rmcp 2.1.0 fixed the leak only in its own `default_http_client()`. Archon
//! builds its own `reqwest::Client` in `create_http_transport` and hands it to
//! `StreamableHttpClientTransport::with_client`, so the upstream fix does not
//! reach archon's path by itself. This test drives that exact path: an MCP
//! endpoint answers with a 307 to a different origin, and the custom header
//! must reach the first server but never the redirect target.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archon_mcp::http_transport::create_http_transport;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header::LOCATION};
use axum::response::IntoResponse;
use axum::routing::post;

const API_KEY_HEADER: &str = "x-api-key";
const API_KEY_VALUE: &str = "archon-redirect-secret";

type Captured = Arc<Mutex<Vec<Option<String>>>>;

#[derive(Clone)]
struct RedirectState {
    location: String,
    seen: Captured,
}

fn record(headers: &HeaderMap, seen: &Captured) {
    let value = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    seen.lock().expect("capture lock").push(value);
}

async fn redirect_handler(
    State(state): State<RedirectState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    record(&headers, &state.seen);
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(LOCATION, state.location)],
        "",
    )
}

async fn target_handler(State(seen): State<Captured>, headers: HeaderMap) -> impl IntoResponse {
    record(&headers, &seen);
    (StatusCode::BAD_REQUEST, "redirect target reached")
}

async fn bind() -> (tokio::net::TcpListener, SocketAddr) {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    (listener, addr)
}

#[tokio::test]
async fn http_transport_does_not_forward_custom_headers_to_redirect_target() {
    let target_seen: Captured = Arc::default();
    let (target_listener, target_addr) = bind().await;
    let target_app = Router::new()
        .route("/capture", post(target_handler))
        .with_state(target_seen.clone());
    let target = tokio::spawn(async move { axum::serve(target_listener, target_app).await });

    let origin_seen: Captured = Arc::default();
    let (origin_listener, origin_addr) = bind().await;
    let origin_app = Router::new()
        .route("/mcp", post(redirect_handler))
        .with_state(RedirectState {
            location: format!("http://{target_addr}/capture"),
            seen: origin_seen.clone(),
        });
    let origin = tokio::spawn(async move { axum::serve(origin_listener, origin_app).await });

    let mut headers = HashMap::new();
    headers.insert(API_KEY_HEADER.to_string(), API_KEY_VALUE.to_string());
    let transport = create_http_transport(
        &format!("http://{origin_addr}/mcp"),
        Some(&headers),
        Duration::from_secs(5),
    )
    .expect("build HTTP transport");

    // The initialize POST hits the redirecting origin. Whatever rmcp makes of
    // the 307, the handshake cannot succeed; bound it so a hang fails loudly.
    let outcome = tokio::time::timeout(Duration::from_secs(20), rmcp::serve_client((), transport))
        .await
        .expect("initialize must not hang on a redirect");
    assert!(outcome.is_err(), "initialize against a redirect must fail");

    let origin_values = origin_seen.lock().expect("lock").clone();
    assert!(
        origin_values
            .iter()
            .any(|v| v.as_deref() == Some(API_KEY_VALUE)),
        "the configured server must receive the custom header, saw {origin_values:?}"
    );
    let target_values = target_seen.lock().expect("lock").clone();
    assert!(
        target_values.is_empty(),
        "the redirect target must not be contacted at all, saw {target_values:?}"
    );

    origin.abort();
    target.abort();
}
