//! Server assembly: how the process binds, what the router exposes, and the
//! shell endpoints that serve the SPA itself.
//!
//! Kept apart from `mod.rs`, which owns the configuration types and the shared
//! `AppState` that every handler reads.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method, Uri};
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use super::api::{EffectivePolicySummary, WebApiState};
use super::live::WebLiveManager;
use super::{
    AppState, WebConfig, WebRuntimeHandles, WebRuntimePaths, actions, agents, api, assets, auth,
    board, board_activity, chat, check_auth, cognitive, corpus, docs_actions, evidence, ingest,
    inspect, live, metrics, pipelines, server_shutdown, settings, terminal, uploads,
    uploads_receive, workflows, world,
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
            board: board::WebBoardStore::new(),
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

    // The terminal route is added or not added; there is no handler that
    // refuses. See `terminal::is_available` for why the decision belongs here
    // and not inside a handler — in short, a shell should not be a 403 away.
    let terminal_route = if terminal::is_available(config, &state.api.policy()) {
        tracing::warn!(
            "web: terminal pane enabled (policy.web.allow_web_terminal, loopback bind); \
             the browser can start an archon process on this host"
        );
        Some(Router::new().route("/api/terminal/ws", get(terminal::ws_handler)))
    } else {
        None
    };

    let router = Router::new()
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
        // Not gated on attached mode, unlike `/api/agents/live`: the board is
        // rows in the memory database, not a process-global registry.
        .route("/api/board/runs", get(board::runs_handler))
        .route("/api/board/runs/{run_id}/items", get(board::items_handler))
        // Run-scoped rather than per item: the feed is what happened across the
        // run, and assembling it from item histories would be a query per row.
        .route(
            "/api/board/runs/{run_id}/activity",
            get(board_activity::activity_handler),
        )
        .route(
            "/api/board/items/{item_id}/history",
            get(board::history_handler),
        )
        .route(
            "/api/actions/evaluate",
            post(actions::evaluate_action_handler),
        )
        .route("/api/uploads/policy", get(uploads::policy_handler))
        .route("/api/uploads/intent", post(uploads::intent_handler))
        .route("/api/uploads/file", post(uploads_receive::receive_handler))
        .route("/api/corpus/summary", get(corpus::summary_handler))
        .route("/api/corpus/search", get(corpus::search_handler))
        .route("/api/corpus/source", get(corpus::preview_handler))
        // Separate from `/api/corpus/source` because a PDF is bytes, not the
        // `String` that preview carries. See `corpus_source.rs`.
        .route(
            "/api/corpus/source/bytes",
            get(corpus::source::bytes_handler),
        )
        .route("/api/ingest/summary", get(ingest::summary_handler))
        .route("/api/ingest/run", post(ingest::run_handler))
        .route("/api/ingest/kb", post(ingest::create_kb_handler))
        .route("/api/docs/delete", post(docs_actions::delete_handler))
        .route(
            "/api/index/control",
            post(docs_actions::index_control_handler),
        )
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
        .route("/static/{*path}", get(static_handler));

    let router = match terminal_route {
        Some(route) => router.merge(route),
        None => router,
    };

    router
        .fallback(spa_fallback_handler)
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

/// Content-Security-Policy for the workbench document.
///
/// The workbench renders PDFs the user ingested from arbitrary URLs through
/// PDF.js. A malicious document that finds a way to reach the DOM must not be
/// able to load or run script, so `script-src` is the point of this header and
/// `'unsafe-inline'`/`'unsafe-eval'` are deliberately absent from it.
///
/// - `'wasm-unsafe-eval'` permits `WebAssembly.compile` and nothing else. PDF.js
///   decodes JPX/JBIG2 images with the WASM modules served from
///   `/static/pdfjs/wasm/`; it does not permit `eval` of JavaScript.
/// - `worker-src 'self'` is what keeps `pdf.worker` local. A CDN worker fails
///   here rather than silently working.
/// - `style-src` keeps `'unsafe-inline'` because xterm and uPlot inject style
///   elements at runtime; that is a cosmetic surface, not a script one.
/// - `object-src 'none'` blocks `<embed>`/`<object>`, i.e. the browser's own
///   plugin PDF path, so the sandboxed PDF.js renderer is the only one.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self' 'wasm-unsafe-eval'; \
     worker-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; \
     font-src 'self' data:; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'none'; \
     form-action 'none'; \
     frame-ancestors 'none'";

fn shell_security_headers() -> [(axum::http::HeaderName, &'static str); 4] {
    [
        (
            axum::http::header::CONTENT_SECURITY_POLICY,
            CONTENT_SECURITY_POLICY,
        ),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (axum::http::header::REFERRER_POLICY, "no-referrer"),
        (axum::http::header::X_FRAME_OPTIONS, "DENY"),
    ]
}

async fn index_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match assets::get_asset("index.html") {
        Some(asset) => {
            let html = String::from_utf8_lossy(asset.data.as_ref()).into_owned();
            (shell_security_headers(), Html(html)).into_response()
        }
        None => (StatusCode::NOT_FOUND, "index.html not embedded").into_response(),
    }
}

/// Send an unrouted page path to its hash form instead of answering 404.
///
/// The workbench is hash-routed, so `/ingest` is genuinely not a route this
/// server has -- but it is what a person types, bookmarks, or reaches by
/// refreshing. A 404 there reads as "that page does not exist", which is the
/// wrong answer about a page that does.
///
/// API and asset paths are deliberately exempt. A mistyped endpoint must stay
/// a 404: a client that followed a redirect into HTML would report a parse
/// failure rather than the missing route it actually asked for, which is the
/// harder bug to find. Non-GET methods are 404 for the same reason -- a POST
/// to an unknown path is a caller error, not a navigation.
async fn spa_fallback_handler(method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if method != Method::GET
        || path.starts_with("/api/")
        || path.starts_with("/static/")
        || path.starts_with("/health")
    {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let view = path.trim_matches('/');
    if view.is_empty() {
        return Redirect::temporary("/").into_response();
    }

    // The query survives the hop so a link carrying parameters still arrives
    // with them; the SPA reads its own query from after the fragment.
    let target = match uri.query() {
        Some(query) if !query.is_empty() => format!("/#/{view}?{query}"),
        _ => format!("/#/{view}"),
    };
    Redirect::temporary(&target).into_response()
}

async fn health_handler() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    fn uri(value: &str) -> Uri {
        value.parse().expect("uri")
    }

    fn location(response: &Response) -> Option<String> {
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }

    /// The reported defect: typing or refreshing a workbench URL answered 404
    /// for a page that exists.
    #[tokio::test]
    async fn a_typed_page_path_is_sent_to_its_hash_route() {
        let response = spa_fallback_handler(Method::GET, uri("/ingest")).await;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(location(&response).as_deref(), Some("/#/ingest"));
    }

    #[tokio::test]
    async fn a_query_string_survives_the_hop() {
        let response = spa_fallback_handler(Method::GET, uri("/corpus?doc=abc")).await;

        assert_eq!(location(&response).as_deref(), Some("/#/corpus?doc=abc"));
    }

    /// A mistyped endpoint must stay a 404. Redirecting it into HTML would make
    /// a client report a JSON parse failure instead of the missing route it
    /// actually asked for.
    #[tokio::test]
    async fn an_unknown_api_path_is_still_not_found() {
        let response = spa_fallback_handler(Method::GET, uri("/api/does-not-exist")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(location(&response).is_none());
    }

    #[tokio::test]
    async fn a_missing_asset_is_still_not_found() {
        let response = spa_fallback_handler(Method::GET, uri("/static/gone.js")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A POST to an unknown path is a caller error, not navigation.
    #[tokio::test]
    async fn a_non_get_to_an_unknown_path_is_not_redirected() {
        let response = spa_fallback_handler(Method::POST, uri("/ingest")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
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
            [
                (axum::http::header::CONTENT_TYPE, asset.mime),
                (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            asset.data.as_ref().to_vec(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, format!("not found: {path}")).into_response(),
    }
}
