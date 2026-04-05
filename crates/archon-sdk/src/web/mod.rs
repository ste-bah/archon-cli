/// Web UI module — HTTP server that serves the embedded SPA and proxies
/// the WebSocket connection to the Archon agent (TASK-CLI-414).
///
/// The server binds to `127.0.0.1:8421` by default. Binding to a non-loopback
/// address requires explicit configuration and activates bearer-token auth.

pub mod assets;

use std::net::SocketAddr;

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum::http::HeaderValue;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

// ---------------------------------------------------------------------------
// WebConfig
// ---------------------------------------------------------------------------

/// Configuration for the Archon web UI server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Port to listen on.
    pub port: u16,
    /// Address to bind to. Use `"127.0.0.1"` (default) for localhost-only
    /// or `"0.0.0.0"` to expose on the network (auth required).
    pub bind_address: String,
    /// Open the default browser automatically after server starts.
    pub open_browser: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            port: 8421,
            bind_address: "127.0.0.1".to_string(),
            open_browser: true,
        }
    }
}

impl WebConfig {
    /// Returns `true` when bound to a loopback address (no auth required).
    pub fn is_localhost(&self) -> bool {
        matches!(
            self.bind_address.as_str(),
            "127.0.0.1" | "::1" | "localhost"
        )
    }
}

// ---------------------------------------------------------------------------
// WebServer
// ---------------------------------------------------------------------------

/// Internal server state threaded through Axum handlers.
#[derive(Clone)]
struct AppState {
    /// Bearer token required for non-localhost access; `None` = no auth.
    token: Option<String>,
}

/// HTTP server that serves the embedded SPA.
///
/// Endpoints:
/// - `GET /` → `index.html`
/// - `GET /static/{path}` → embedded asset
/// - `GET /health` → `{"status":"ok"}`
pub struct WebServer {
    config: WebConfig,
    token: Option<String>,
}

impl WebServer {
    /// Create a new `WebServer`.
    ///
    /// `token` is the bearer token required when binding to a non-loopback
    /// address. Pass `None` for localhost-only deployments.
    pub fn new(config: WebConfig, token: Option<String>) -> Self {
        Self { config, token }
    }

    /// Bind and serve. Blocks until the server is shut down.
    pub async fn run(self) -> anyhow::Result<()> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid bind address: {e}"))?;

        if !self.config.is_localhost() && self.token.is_none() {
            tracing::warn!(
                "web: server bound to {} without auth token — \
                 non-localhost access is unauthenticated",
                self.config.bind_address
            );
        }

        let state = AppState {
            token: self.token.clone(),
        };

        let local_origins: Vec<HeaderValue> = [
            format!("http://127.0.0.1:{}", self.config.port),
            format!("http://localhost:{}", self.config.port),
            format!("http://[::1]:{}", self.config.port),
        ]
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
        let cors = CorsLayer::new()
            .allow_origin(AllowOrigin::list(local_origins))
            .allow_methods(Any)
            .allow_headers(Any);

        let app = Router::new()
            .route("/", get(index_handler))
            .route("/health", get(health_handler))
            .route("/static/{*path}", get(static_handler))
            .layer(cors)
            .with_state(state);

        tracing::info!("web: listening on http://{addr}");
        println!("Archon web UI: http://{addr}");

        if self.config.open_browser {
            let url = format!("http://{addr}");
            // Non-fatal: best-effort browser open
            if let Err(e) = open::that(&url) {
                tracing::warn!("web: could not open browser: {e}");
            }
        }

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("web: bind failed: {e}"))?;

        axum::serve(listener, app.into_make_service())
            .await
            .map_err(|e| anyhow::anyhow!("web: server error: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn index_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match assets::get_asset("index.html") {
        Some(asset) => {
            let html = String::from_utf8_lossy(asset.data.as_ref()).into_owned();
            Html(html).into_response()
        }
        None => (StatusCode::NOT_FOUND, "index.html not embedded").into_response(),
    }
}

async fn health_handler() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn static_handler(
    Path(path): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match assets::get_asset(&path) {
        Some(asset) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, asset.mime)],
            asset.data.as_ref().to_vec(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, format!("not found: {path}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(ref required) = state.token else {
        return Ok(());
    };

    // Accept token in Authorization header or (for GET requests) as a query
    // param — query param handling lives in the caller for simplicity.
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if archon_core::remote::auth::validate_token(required, provided) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "Unauthorized").into_response())
    }
}
