/// Web UI module — HTTP server that serves the embedded SPA and proxies
/// the WebSocket connection to the Archon agent (TASK-CLI-414).
///
/// The server binds to `127.0.0.1:8421` by default. Binding to a non-loopback
/// address requires explicit configuration and activates bearer-token auth.
pub mod actions;
pub mod api;
pub mod assets;
pub mod auth;
pub mod chat;
pub mod corpus;
pub mod evidence;
pub mod ingest;
mod ingest_jobs;
mod ingest_store;
pub mod inspect;
pub mod live;
pub mod metrics;
pub mod pipelines;
pub mod settings;
pub mod uploads;
pub mod world;
mod world_jepa;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use api::{EffectivePolicySummary, WebApiState};
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use live::WebLiveManager;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub use api::{WebPolicySummary, WebSubsystemPolicySummary};

#[derive(Debug, Clone)]
pub struct WebRuntimePaths {
    pub cwd: PathBuf,
    pub archon_home: PathBuf,
    pub archon_data: PathBuf,
    pub memory_db: PathBuf,
    pub session_db: PathBuf,
    pub session_activity_root: PathBuf,
    pub world_model_root: PathBuf,
    pub reasoning_quality_root: PathBuf,
}

impl WebRuntimePaths {
    pub fn from_overrides(memory_path: Option<&str>, session_db: Option<&str>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let archon_home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".archon");
        let archon_data = std::env::var("ARCHON_DATA_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(default_archon_data_dir);
        let memory_db = memory_path
            .map(resolve_memory_path)
            .unwrap_or_else(|| archon_data.join("memory.db"));
        let session_db = session_db
            .map(PathBuf::from)
            .unwrap_or_else(|| archon_data.join("sessions").join("sessions.db"));
        Self {
            cwd,
            archon_home: archon_home.clone(),
            archon_data,
            memory_db,
            session_db,
            session_activity_root: archon_home.join("sessions"),
            world_model_root: archon_home.join("world-model"),
            reasoning_quality_root: archon_home.join("reasoning-quality"),
        }
    }
}

impl Default for WebRuntimePaths {
    fn default() -> Self {
        Self::from_overrides(None, None)
    }
}

fn default_archon_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
}

fn resolve_memory_path(value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.extension().is_some_and(|ext| ext == "db") {
        path
    } else {
        path.join("memory.db")
    }
}

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
    /// Maximum accepted HTTP request body size for mutating web APIs.
    pub max_body_bytes: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            port: 8421,
            bind_address: "127.0.0.1".to_string(),
            open_browser: true,
            max_body_bytes: 64 * 1024 * 1024,
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
pub(crate) struct AppState {
    /// Bearer token required for non-localhost access; `None` = no auth.
    token: Option<String>,
    /// Read-only API state surfaced to the web workbench shell.
    api: WebApiState,
    /// Bounded live event buffer used by the web workbench.
    live: WebLiveManager,
    /// Runtime storage locations resolved from the active config.
    paths: WebRuntimePaths,
    /// Optional chat/session bridge supplied by the binary runtime.
    chat_backend: Option<Arc<dyn chat::WebChatBackend>>,
    /// In-memory ingest job state for web-triggered document/KB/video operations.
    ingest_jobs: ingest::WebIngestJobStore,
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
    policy: EffectivePolicySummary,
    paths: WebRuntimePaths,
    chat_backend: Option<Arc<dyn chat::WebChatBackend>>,
    unsafe_allow_unauthenticated_nonlocal_bind: bool,
}

impl WebServer {
    /// Create a new `WebServer`.
    ///
    /// `token` is the bearer token required when binding to a non-loopback
    /// address. Pass `None` for localhost-only deployments.
    pub fn new(config: WebConfig, token: Option<String>) -> Self {
        Self::with_policy(config, token, EffectivePolicySummary::default_safe())
    }

    pub fn with_policy(
        config: WebConfig,
        token: Option<String>,
        policy: EffectivePolicySummary,
    ) -> Self {
        Self {
            config,
            token,
            policy,
            paths: WebRuntimePaths::default(),
            chat_backend: None,
            unsafe_allow_unauthenticated_nonlocal_bind: false,
        }
    }

    pub fn with_policy_and_paths(
        config: WebConfig,
        token: Option<String>,
        policy: EffectivePolicySummary,
        paths: WebRuntimePaths,
    ) -> Self {
        Self {
            config,
            token,
            policy,
            paths,
            chat_backend: None,
            unsafe_allow_unauthenticated_nonlocal_bind: false,
        }
    }

    pub fn with_chat_backend(mut self, backend: Arc<dyn chat::WebChatBackend>) -> Self {
        self.chat_backend = Some(backend);
        self
    }

    /// UNSAFE: allow a non-loopback bind without bearer-token auth.
    ///
    /// This is intentionally not a `WebConfig` field so it cannot be enabled
    /// silently through persistent config. The CLI exposes it as an explicit,
    /// noisy operator override.
    pub fn unsafe_allow_unauthenticated_nonlocal_bind_for_cli(mut self) -> Self {
        self.unsafe_allow_unauthenticated_nonlocal_bind = true;
        self
    }

    /// Bind and serve. Blocks until the server is shut down.
    pub async fn run(self) -> anyhow::Result<()> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid bind address: {e}"))?;

        let token = validate_bind_auth(
            &self.config,
            &self.token,
            self.unsafe_allow_unauthenticated_nonlocal_bind,
        )?;

        if !self.config.is_localhost()
            && token.is_none()
            && self.unsafe_allow_unauthenticated_nonlocal_bind
        {
            tracing::warn!(
                "web: UNSAFE override enabled: server bound to {} without auth token; \
                 non-localhost access is unauthenticated",
                self.config.bind_address
            );
        }

        let live = WebLiveManager::new(1024);
        live.record("web.runtime.started", "Archon web workbench started");

        let state = AppState {
            token: token.clone(),
            api: WebApiState::from_server_config(
                &self.config,
                token.is_some(),
                self.policy.clone(),
                self.paths.clone(),
            ),
            live,
            paths: self.paths.clone(),
            chat_backend: self.chat_backend.clone(),
            ingest_jobs: ingest::new_job_store(),
        };

        let app = build_app(&self.config, state);

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

fn build_app(config: &WebConfig, state: AppState) -> Router {
    let local_origins: Vec<HeaderValue> = [
        format!("http://127.0.0.1:{}", config.port),
        format!("http://localhost:{}", config.port),
        format!("http://[::1]:{}", config.port),
    ]
    .iter()
    .filter_map(|origin| origin.parse().ok())
    .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(local_origins))
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .route("/api/status", get(api::status_handler))
        .route("/api/auth/session", get(auth::session_handler))
        .route("/api/auth/logout", post(auth::logout_handler))
        .route(
            "/api/chat/submit",
            post(chat::submit_handler).layer(DefaultBodyLimit::max(config.max_body_bytes)),
        )
        .route("/api/chat/history", get(chat::history_handler))
        .route("/api/config/effective", get(api::config_handler))
        .route("/api/policy/effective", get(api::policy_handler))
        .route("/api/live/snapshot", get(live::snapshot_handler))
        .route(
            "/api/actions/evaluate",
            post(actions::evaluate_action_handler),
        )
        .route("/api/uploads/policy", get(uploads::policy_handler))
        .route("/api/uploads/intent", post(uploads::intent_handler))
        .route("/api/corpus/summary", get(corpus::summary_handler))
        .route("/api/corpus/search", get(corpus::search_handler))
        .route("/api/corpus/source", get(corpus::preview_handler))
        .route("/api/ingest/summary", get(ingest::summary_handler))
        .route("/api/ingest/run", post(ingest::run_handler))
        .route("/api/ingest/kb", post(ingest::create_kb_handler))
        .route("/api/learning/summary", get(inspect::learning_handler))
        .route("/api/world/summary", get(world::summary_handler))
        .route("/api/pipelines/summary", get(pipelines::summary_handler))
        .route("/api/metrics/summary", get(metrics::summary_handler))
        .route("/api/evidence/graph", get(evidence::graph_handler))
        .route("/api/settings/summary", get(inspect::settings_handler))
        .route(
            "/api/settings/theme-profile",
            get(settings::theme_profile_handler).post(settings::save_theme_profile_handler),
        )
        .route("/static/{*path}", get(static_handler))
        .layer(cors)
        .with_state(state)
}

fn validate_bind_auth(
    config: &WebConfig,
    token: &Option<String>,
    unsafe_allow_unauthenticated_nonlocal_bind: bool,
) -> anyhow::Result<Option<String>> {
    let token = token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if config.is_localhost() || token.is_some() {
        return Ok(token);
    }

    if unsafe_allow_unauthenticated_nonlocal_bind {
        return Ok(None);
    }

    anyhow::bail!(
        "web: refusing to bind {} without an auth token; use localhost, provide a token, \
         or pass the explicit CLI-only --allow-unauthenticated-nonlocal-bind override",
        config.bind_address
    );
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn index_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
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

#[allow(clippy::result_large_err)]
pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(ref required) = state.token else {
        return Ok(());
    };

    // Accept bearer tokens from the Authorization header only. Query-string
    // tokens are intentionally not accepted here so tokens do not leak through
    // URLs or request logs.
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use tower::ServiceExt;

    fn test_state(config: &WebConfig, token: Option<String>) -> AppState {
        let auth_required = token.is_some();
        let paths = WebRuntimePaths::default();
        AppState {
            token,
            api: WebApiState::from_server_config(
                config,
                auth_required,
                EffectivePolicySummary::default_safe(),
                paths.clone(),
            ),
            live: WebLiveManager::new(16),
            paths,
            chat_backend: None,
            ingest_jobs: ingest::new_job_store(),
        }
    }

    #[test]
    fn runtime_paths_use_memory_database_file() {
        let paths = WebRuntimePaths::from_overrides(None, None);
        assert!(paths.memory_db.ends_with("memory.db"));
        assert!(!paths.memory_db.ends_with(".archon/memory"));
    }

    #[test]
    fn runtime_paths_honor_explicit_memory_file() {
        let paths = WebRuntimePaths::from_overrides(Some("/tmp/custom-memory.db"), None);
        assert_eq!(paths.memory_db, PathBuf::from("/tmp/custom-memory.db"));
    }

    #[test]
    fn nonlocal_bind_without_token_is_rejected_by_default() {
        let config = WebConfig {
            bind_address: "0.0.0.0".to_string(),
            ..WebConfig::default()
        };
        let err = validate_bind_auth(&config, &None, false).unwrap_err();
        assert!(err.to_string().contains("refusing to bind"));
    }

    #[test]
    fn nonlocal_bind_with_blank_token_is_rejected_by_default() {
        let config = WebConfig {
            bind_address: "0.0.0.0".to_string(),
            ..WebConfig::default()
        };
        let err = validate_bind_auth(&config, &Some("   ".to_string()), false).unwrap_err();
        assert!(err.to_string().contains("refusing to bind"));
    }

    #[test]
    fn nonlocal_bind_with_token_is_allowed() {
        let config = WebConfig {
            bind_address: "0.0.0.0".to_string(),
            ..WebConfig::default()
        };
        let token = validate_bind_auth(&config, &Some("secret-token".to_string()), false).unwrap();
        assert_eq!(token.as_deref(), Some("secret-token"));
    }

    #[test]
    fn nonlocal_bind_without_token_requires_explicit_cli_unsafe_override() {
        let config = WebConfig {
            bind_address: "0.0.0.0".to_string(),
            ..WebConfig::default()
        };
        let token = validate_bind_auth(&config, &None, true).unwrap();
        assert!(token.is_none());
    }

    #[tokio::test]
    async fn chat_submit_requires_bearer_auth_when_token_is_configured() {
        let config = WebConfig {
            bind_address: "0.0.0.0".to_string(),
            open_browser: false,
            ..WebConfig::default()
        };
        let app = build_app(&config, test_state(&config, Some("secret-token".into())));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat/submit")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"message":"hello","attachments":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn chat_submit_body_limit_is_configurable() {
        let config = WebConfig {
            open_browser: false,
            max_body_bytes: 16,
            ..WebConfig::default()
        };
        let app = build_app(&config, test_state(&config, None));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat/submit")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"message":"this body is too large","attachments":[]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
