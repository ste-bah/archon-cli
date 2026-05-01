//! MCP server lifecycle management.
//!
//! `McpServerManager` manages multiple MCP server instances, handling
//! startup, health tracking, automatic restarts with exponential backoff,
//! graceful shutdown, and tool aggregation.
//!
//! Module layout (#204 HYGIENE-MCP-FILE-SIZES split):
//!   * this `mod.rs` — `McpServerManager` + state + backoff + Default
//!   * `connect.rs`  — per-transport dispatch (stdio/http/ws/sse) + WS adapter
//!     + the `#[doc(hidden)] pub` `connect_server_for_test`
//!       wrapper used by integration tests
//!   * `tests.rs`    — unit tests for the Manager + backoff

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::client::McpClient;
use crate::types::{McpError, ServerConfig, ServerState};

mod connect;
#[cfg(test)]
mod tests;

pub use connect::connect_server_for_test;

/// Default connect timeout for HTTP-family transports (http/ws/sse).
pub(crate) const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of automatic restart attempts before giving up.
const MAX_RESTART_ATTEMPTS: u32 = 5;

/// Base delay for exponential backoff (doubles each attempt).
const BASE_BACKOFF: Duration = Duration::from_secs(1);

/// Cap on backoff delay.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Tracked state for a single managed server.
struct ManagedServer {
    config: ServerConfig,
    state: ServerState,
    client: Option<Arc<McpClient>>,
    restart_count: u32,
}

/// Path to the file where disabled server names are persisted.
fn disabled_names_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("mcp_disabled.json")
}

/// Load disabled server names from disk. Returns empty set on any error.
fn load_disabled_names() -> HashSet<String> {
    let path = disabled_names_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<Vec<String>>(&content)
            .unwrap_or_default()
            .into_iter()
            .collect(),
        Err(_) => HashSet::new(),
    }
}

/// Persist the disabled server names set to disk.
fn save_disabled_names(names: &HashSet<String>) {
    let path = disabled_names_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let list: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    if let Ok(json) = serde_json::to_string(&list) {
        let _ = std::fs::write(&path, json);
    }
}

/// Manages the lifecycle of multiple MCP server processes.
#[derive(Clone)]
pub struct McpServerManager {
    servers: Arc<RwLock<HashMap<String, ManagedServer>>>,
    /// Names of servers that have been explicitly disabled (persisted to disk).
    disabled_names: Arc<RwLock<HashSet<String>>>,
}

impl McpServerManager {
    /// Create a new empty manager. Loads disabled names from disk.
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            disabled_names: Arc::new(RwLock::new(load_disabled_names())),
        }
    }

    /// Start all servers from the given configs.
    ///
    /// Each server is started independently; one failing does not
    /// prevent others from starting. Failures are logged and the
    /// server's state is set to `Crashed`.
    pub async fn start_all(&self, configs: Vec<ServerConfig>) -> Vec<McpError> {
        let mut errors = Vec::new();

        for config in configs {
            if let Err(e) = self.start_server(config).await {
                errors.push(e);
            }
        }

        errors
    }

    /// Start a single server and register it with the manager.
    async fn start_server(&self, config: ServerConfig) -> Result<(), McpError> {
        let name = config.name.clone();
        tracing::info!(server = %name, "starting MCP server");

        // Register as Starting
        {
            let mut servers = self.servers.write().await;
            servers.insert(
                name.clone(),
                ManagedServer {
                    config: config.clone(),
                    state: ServerState::Starting,
                    client: None,
                    restart_count: 0,
                },
            );
        }

        match connect::connect_server(&config).await {
            Ok(client) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(&name) {
                    entry.state = ServerState::Ready;
                    entry.client = Some(Arc::new(client));
                }
                tracing::info!(server = %name, "MCP server ready");
                Ok(())
            }
            Err(e) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(&name) {
                    entry.state = ServerState::Crashed;
                }
                tracing::error!(server = %name, error = %e, "MCP server failed to start");
                Err(e)
            }
        }
    }

    /// Attempt to restart a crashed server with exponential backoff.
    ///
    /// Returns an error if the server is not found or max restarts
    /// have been exceeded.
    pub async fn restart_server(&self, name: &str) -> Result<(), McpError> {
        let (cfg, delay) = {
            let mut servers = self.servers.write().await;
            let entry = servers
                .get_mut(name)
                .ok_or_else(|| McpError::ServerNotFound(name.into()))?;

            if entry.restart_count >= MAX_RESTART_ATTEMPTS {
                entry.state = ServerState::Stopped;
                return Err(McpError::MaxRestartsExceeded(name.into()));
            }

            entry.restart_count += 1;
            entry.state = ServerState::Restarting;
            entry.client = None;

            let delay = backoff_delay(entry.restart_count);
            tracing::info!(
                server = %name,
                attempt = entry.restart_count,
                delay_secs = delay.as_secs(),
                "restarting MCP server"
            );

            (entry.config.clone(), delay)
        };
        tokio::time::sleep(delay).await;

        match connect::connect_server(&cfg).await {
            Ok(client) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(name) {
                    entry.state = ServerState::Ready;
                    entry.client = Some(Arc::new(client));
                    entry.restart_count = 0;
                }
                tracing::info!(server = %name, "MCP server restarted successfully");
                Ok(())
            }
            Err(e) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(name) {
                    entry.state = ServerState::Crashed;
                }
                tracing::error!(server = %name, error = %e, "MCP server restart failed");
                Err(e)
            }
        }
    }

    /// Disable a server: stop it and persist disabled=true to disk.
    /// The server will not auto-reconnect until `enable_server` is called.
    pub async fn disable_server(&self, name: &str) -> Result<(), McpError> {
        // Mark the server state as Stopped and drop the client reference.
        {
            let mut servers = self.servers.write().await;
            if let Some(entry) = servers.get_mut(name) {
                entry.state = ServerState::Stopped;
                // Drop the client Arc so the underlying connection is freed.
                entry.client = None;
            }
            // If the server is not in the map we still track the disabled name.
        }

        // Add to disabled set and persist.
        {
            let mut disabled = self.disabled_names.write().await;
            disabled.insert(name.to_string());
            save_disabled_names(&disabled);
        }

        tracing::info!(server = %name, "MCP server disabled");
        Ok(())
    }

    /// Enable a server: remove from the disabled set and attempt to start it.
    pub async fn enable_server(&self, name: &str) -> Result<(), McpError> {
        // Retrieve the config before we drop the lock.
        let cfg_opt = {
            let servers = self.servers.read().await;
            servers.get(name).map(|e| e.config.clone())
        };

        // Remove from disabled set and persist.
        {
            let mut disabled = self.disabled_names.write().await;
            disabled.remove(name);
            save_disabled_names(&disabled);
        }

        // Restart if we have a config for this server.
        if let Some(cfg) = cfg_opt {
            tracing::info!(server = %name, "MCP server enabled — restarting");
            self.start_server(cfg).await?;
        } else {
            tracing::info!(server = %name, "MCP server enabled (no config to restart)");
        }

        Ok(())
    }

    /// Return (server_name, ServerState, is_disabled) for all known servers.
    ///
    /// Servers that are only in the disabled set (never started) also appear
    /// with `ServerState::Stopped`.
    pub async fn get_server_info(&self) -> Vec<(String, ServerState, bool)> {
        let servers = self.servers.read().await;
        let disabled = self.disabled_names.read().await;

        let mut result: Vec<(String, ServerState, bool)> = servers
            .iter()
            .map(|(name, entry)| {
                let is_disabled = disabled.contains(name.as_str());
                (name.clone(), entry.state, is_disabled)
            })
            .collect();

        for dname in disabled.iter() {
            if !servers.contains_key(dname.as_str()) {
                result.push((dname.clone(), ServerState::Stopped, true));
            }
        }

        result
    }

    /// Return the current state of each managed server.
    pub async fn get_server_states(&self) -> HashMap<String, ServerState> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, s)| (name.clone(), s.state))
            .collect()
    }

    /// List tool names for a single Ready server, as qualified `mcp__server__tool` strings.
    /// Returns an empty vec if the server is not Ready or not found.
    pub async fn list_tools_for(&self, server_name: &str) -> Vec<String> {
        let servers = self.servers.read().await;
        let Some(entry) = servers.get(server_name) else {
            return Vec::new();
        };
        if entry.state != ServerState::Ready {
            return Vec::new();
        }
        let Some(ref client) = entry.client else {
            return Vec::new();
        };
        match client.list_tools().await {
            Ok(tools) => tools
                .iter()
                .map(|t| format!("mcp__{}_{}", server_name, t.name))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Build [`crate::tool_bridge::McpTool`] instances for all tools on all
    /// Ready servers. Returns a `Vec` ready to be boxed and registered into a
    /// `ToolRegistry`.
    pub async fn build_mcp_tools(&self) -> Vec<crate::tool_bridge::McpTool> {
        let servers = self.servers.read().await;
        let mut tools = Vec::new();
        for (server_name, entry) in servers.iter() {
            if entry.state != ServerState::Ready {
                continue;
            }
            if let Some(ref client) = entry.client {
                match client.list_tools().await {
                    Ok(tool_defs) => {
                        for tool_def in tool_defs {
                            tools.push(crate::tool_bridge::McpTool::new(
                                server_name,
                                tool_def,
                                Arc::clone(client),
                            ));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            server = %server_name,
                            error = %e,
                            "failed to list tools for registry"
                        );
                    }
                }
            }
        }
        tools
    }

    /// Gracefully shut down all managed servers.
    ///
    /// Each server receives a shutdown signal. Errors are logged
    /// but do not prevent other servers from shutting down.
    pub async fn shutdown_all(&self) -> Vec<McpError> {
        let mut errors = Vec::new();
        let mut servers = self.servers.write().await;

        let names: Vec<String> = servers.keys().cloned().collect();
        for name in names {
            if let Some(mut entry) = servers.remove(&name) {
                if let Some(arc_client) = entry.client.take() {
                    tracing::info!(server = %name, "shutting down MCP server");
                    match Arc::try_unwrap(arc_client) {
                        Ok(client) => {
                            if let Err(e) = client.shutdown().await {
                                tracing::error!(server = %name, error = %e, "shutdown error");
                                errors.push(e);
                            }
                        }
                        Err(_arc) => {
                            tracing::warn!(
                                server = %name,
                                "McpClient Arc has other owners; skipping graceful shutdown"
                            );
                        }
                    }
                }
                entry.state = ServerState::Stopped;
            }
        }

        errors
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate exponential backoff delay capped at [`MAX_BACKOFF`].
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(63);
    let secs = BASE_BACKOFF.as_secs().saturating_mul(1u64 << shift);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}
