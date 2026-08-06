//! Server assembly: how the process binds, what the router exposes, and the
//! shell endpoints that serve the SPA itself.
//!
//! Kept apart from `mod.rs`, which owns the configuration types and the shared
//! `AppState` that every handler reads.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use super::api::{EffectivePolicySummary, WebApiState};
use super::live::WebLiveManager;
use super::{
    AppState, WebConfig, WebRuntimeHandles, WebRuntimePaths, actions, agents, api, assets, auth,
    chat, check_auth, cognitive, corpus, evidence, ingest, inspect, live, metrics, pipelines,
    server_shutdown, settings, uploads, workflows, world,
};

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
    handles: WebRuntimeHandles,
    attached: bool,
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
        Self::with_policy_and_paths(config, token, policy, WebRuntimePaths::default())
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
            handles: WebRuntimeHandles::default(),
            attached: false,
        }
    }

    /// Server that runs inside a host process and observes it.
    ///
    /// Differs from `with_policy_and_paths` in two ways that matter: it reuses
    /// the host's open handles instead of opening its own, and it reports
    /// itself as attached so `/api/agents/live` — which reads process-global
    /// registries that cannot cross a process boundary — is meaningful.
    ///
    /// No chat backend is attached here on purpose: the host owns the
    /// conversation loop, and a second `WebChatBridge` would be a separate
    /// session masquerading as this one.
    pub fn attached(
        config: WebConfig,
        token: Option<String>,
        policy: EffectivePolicySummary,
        paths: WebRuntimePaths,
        handles: WebRuntimeHandles,
    ) -> Self {
        Self {
            handles,
            attached: true,
            ..Self::with_policy_and_paths(config, token, policy, paths)
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
        self.run_until(std::future::pending()).await
    }

    pub async fn run_until<F>(self, shutdown: F) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
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

        let live = self
            .handles
            .live
            .clone()
            .unwrap_or_else(|| WebLiveManager::new(1024));
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
            handles: self.handles.clone(),
            agents: agents::WebAgentObserver::new(),
            attached: self.attached,
        };

        let app = build_app(&self.config, state);

        tracing::info!("web: listening on http://{addr}");
        if !self.attached {
            // Attached mode shares a terminal with the TUI, which owns the
            // alternate screen — a stray println! corrupts the display. The
            // slash command reports the URL through the TUI instead.
            println!("Archon web UI: http://{addr}");
        }

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

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = axum::serve(listener, app.into_make_service()).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        server_shutdown::await_shutdown(server, shutdown, shutdown_tx).await
    }
}

pub(super) fn build_app(config: &WebConfig, state: AppState) -> Router {
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
        .route("/api/live/stream", get(live::stream_handler))
        .route("/api/agents/live", get(agents::live_handler))
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
        .route("/api/cognitive/summary", get(cognitive::summary_handler))
        .route("/api/world/summary", get(world::summary_handler))
        .route("/api/pipelines/summary", get(pipelines::summary_handler))
        .route("/api/workflows/summary", get(workflows::summary_handler))
        .route("/api/workflows/control", post(workflows::control_handler))
        .route("/api/workflows/{run_id}", get(workflows::detail_handler))
        .route(
            "/api/workflows/{run_id}/events",
            get(workflows::events_handler),
        )
        .route(
            "/api/workflows/{run_id}/stream",
            get(workflows::stream_handler),
        )
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

pub(super) fn validate_bind_auth(
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
// Shell handlers
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
