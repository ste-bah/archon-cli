/// Web UI module — HTTP server that serves the embedded SPA and proxies
/// the WebSocket connection to the Archon agent (TASK-CLI-414).
///
/// The server binds to `127.0.0.1:8421` by default. Binding to a non-loopback
/// address requires explicit configuration and activates bearer-token auth.
pub mod actions;
pub mod agents;
pub mod api;
pub mod assets;
pub mod auth;
pub mod board;
pub mod board_activity;
pub mod chat;
pub mod cognitive;
pub mod corpus;
pub mod evidence;
pub mod ingest;
mod ingest_jobs;
mod ingest_store;
pub mod inspect;
pub mod live;
pub mod metrics;
pub mod pipelines;
mod server;
mod server_shutdown;
pub mod settings;
pub mod uploads;
pub mod workflows;
pub mod world;
mod world_jepa;

use std::path::PathBuf;
use std::sync::Arc;

use api::WebApiState;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use live::WebLiveManager;

pub use api::{WebPolicySummary, WebSubsystemPolicySummary};
pub use server::WebServer;

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

/// Already-open state handed to a server running inside a host process.
///
/// `with_policy_and_paths` makes the server open everything itself, which is
/// exactly what makes `archon web` a second independent client of the same
/// files. An attached server takes the host's handles instead, so it reports
/// on the session that is actually running.
#[derive(Clone, Default)]
pub struct WebRuntimeHandles {
    /// Live event buffer owned by the host. Anything the host records shows up
    /// in the dashboard feed. `None` makes the server allocate its own.
    pub live: Option<live::WebLiveManager>,
    /// The host's already-open memory store, used instead of opening a second
    /// connection to the same database file on every request.
    pub memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
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
// Shared handler state
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
    /// Already-open host handles; empty for a standalone `archon web`.
    handles: WebRuntimeHandles,
    /// Elapsed-time bookkeeping for the live agent projection.
    agents: agents::WebAgentObserver,
    /// Lazily-opened reader for the task board. Not in `handles`: the board is
    /// reachable from any process that can open the database, so it does not
    /// depend on the host passing anything in.
    board: board::WebBoardStore,
    /// `true` when this server runs inside the session it reports on.
    attached: bool,
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
#[path = "mod_tests.rs"]
mod tests;
