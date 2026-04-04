//! MCP server lifecycle management.
//!
//! `McpServerManager` manages multiple MCP server instances, handling
//! startup, health tracking, automatic restarts with exponential backoff,
//! graceful shutdown, and tool aggregation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::client::McpClient;
use crate::http_transport::create_http_transport;
use crate::transport::spawn_transport;
use crate::types::{McpError, McpToolDef, ServerConfig, ServerState};

/// Default connect timeout for HTTP transports.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
    client: Option<McpClient>,
    restart_count: u32,
}

/// Manages the lifecycle of multiple MCP server processes.
#[derive(Clone)]
pub struct McpServerManager {
    servers: Arc<RwLock<HashMap<String, ManagedServer>>>,
}

impl McpServerManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
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

        match connect_server(&config).await {
            Ok(client) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(&name) {
                    entry.state = ServerState::Ready;
                    entry.client = Some(client);
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

        match connect_server(&cfg).await {
            Ok(client) => {
                let mut servers = self.servers.write().await;
                if let Some(entry) = servers.get_mut(name) {
                    entry.state = ServerState::Ready;
                    entry.client = Some(client);
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

    /// Return the current state of each managed server.
    pub async fn get_server_states(&self) -> HashMap<String, ServerState> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, s)| (name.clone(), s.state))
            .collect()
    }

    /// Aggregate tools from all servers in the `Ready` state.
    pub async fn list_all_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let servers = self.servers.read().await;
        let mut all_tools = Vec::new();

        for (name, entry) in servers.iter() {
            if entry.state != ServerState::Ready {
                continue;
            }
            if let Some(ref client) = entry.client {
                match client.list_tools().await {
                    Ok(tools) => all_tools.extend(tools),
                    Err(e) => {
                        tracing::warn!(
                            server = %name,
                            error = %e,
                            "failed to list tools"
                        );
                    }
                }
            }
        }

        Ok(all_tools)
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<crate::types::McpToolResult, McpError> {
        let servers = self.servers.read().await;
        let entry = servers
            .get(server_name)
            .ok_or_else(|| McpError::ServerNotFound(server_name.into()))?;

        if entry.state != ServerState::Ready {
            return Err(McpError::ServerNotReady(
                server_name.into(),
                entry.state,
            ));
        }

        let client = entry
            .client
            .as_ref()
            .ok_or_else(|| McpError::ServerNotReady(
                server_name.into(),
                entry.state,
            ))?;

        client.call_tool(tool_name, arguments).await
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
                if let Some(client) = entry.client.take() {
                    tracing::info!(server = %name, "shutting down MCP server");
                    if let Err(e) = client.shutdown().await {
                        tracing::error!(server = %name, error = %e, "shutdown error");
                        errors.push(e);
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

/// Create an appropriate transport and initialize an MCP client for a server config.
///
/// Routes to stdio or HTTP transport based on `config.transport`.
async fn connect_server(config: &ServerConfig) -> Result<McpClient, McpError> {
    match config.transport.as_str() {
        "http" => {
            let url = config.url.as_deref().ok_or_else(|| {
                McpError::Transport(format!(
                    "server '{}' has transport=http but no url configured",
                    config.name
                ))
            })?;
            let transport = create_http_transport(
                url,
                config.headers.as_ref(),
                HTTP_CONNECT_TIMEOUT,
            )?;
            McpClient::initialize(config, transport).await
        }
        "stdio" | "" => {
            let transport = spawn_transport(config)?;
            McpClient::initialize(config, transport).await
        }
        other => {
            tracing::warn!(
                server = %config.name,
                transport = %other,
                "unknown transport type, skipping server"
            );
            Err(McpError::Transport(format!(
                "unknown transport type '{}' for server '{}'",
                other, config.name
            )))
        }
    }
}

/// Calculate exponential backoff delay capped at [`MAX_BACKOFF`].
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(63);
    let secs = BASE_BACKOFF.as_secs().saturating_mul(1u64 << shift);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_values() {
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(4), Duration::from_secs(8));
        assert_eq!(backoff_delay(5), Duration::from_secs(16));
        // After cap
        assert_eq!(backoff_delay(10), Duration::from_secs(60));
        assert_eq!(backoff_delay(100), Duration::from_secs(60));
    }

    #[test]
    fn backoff_delay_zero() {
        // Edge case: attempt 0 should not panic
        let d = backoff_delay(0);
        assert!(d <= MAX_BACKOFF);
    }

    #[tokio::test]
    async fn manager_new_is_empty() {
        let mgr = McpServerManager::new();
        let states = mgr.get_server_states().await;
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn manager_start_bad_server_records_crash() {
        let mgr = McpServerManager::new();
        let config = ServerConfig {
            name: "bad-server".into(),
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
        };

        let errors = mgr.start_all(vec![config]).await;
        assert!(!errors.is_empty());

        let states = mgr.get_server_states().await;
        assert_eq!(states.get("bad-server"), Some(&ServerState::Crashed));
    }

    #[tokio::test]
    async fn manager_shutdown_empty_is_ok() {
        let mgr = McpServerManager::new();
        let errors = mgr.shutdown_all().await;
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn manager_call_tool_server_not_found() {
        let mgr = McpServerManager::new();
        let result = mgr.call_tool("nonexistent", "tool", None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            McpError::ServerNotFound(name) => assert_eq!(name, "nonexistent"),
            other => panic!("expected ServerNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn manager_list_all_tools_empty() {
        let mgr = McpServerManager::new();
        let tools = mgr.list_all_tools().await.expect("should succeed");
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn manager_restart_unknown_server() {
        let mgr = McpServerManager::new();
        let result = mgr.restart_server("unknown").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            McpError::ServerNotFound(name) => assert_eq!(name, "unknown"),
            other => panic!("expected ServerNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn manager_default_trait() {
        let mgr = McpServerManager::default();
        let states = mgr.get_server_states().await;
        assert!(states.is_empty());
    }
}
