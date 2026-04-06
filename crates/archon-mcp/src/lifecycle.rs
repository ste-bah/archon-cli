//! MCP server lifecycle management.
//!
//! `McpServerManager` manages multiple MCP server instances, handling
//! startup, health tracking, automatic restarts with exponential backoff,
//! graceful shutdown, and tool aggregation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::client::McpClient;
use crate::http_transport::create_http_transport;
use crate::transport::spawn_transport;
use crate::transport_ws::WebSocketTransport;
use crate::types::{McpError, ServerConfig, ServerState};
use crate::ws_config::{WsConfig, WsReconnectConfig};

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

        match connect_server(&config).await {
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

        match connect_server(&cfg).await {
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

        // Start with all servers in the map.
        let mut result: Vec<(String, ServerState, bool)> = servers
            .iter()
            .map(|(name, entry)| {
                let is_disabled = disabled.contains(name.as_str());
                (name.clone(), entry.state, is_disabled)
            })
            .collect();

        // Also include any disabled names not yet in the servers map.
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
                    // McpClient::shutdown() takes self, so we unwrap the Arc.
                    // If other clones exist (e.g. McpTool still holds one),
                    // we skip the graceful shutdown to avoid blocking.
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
            let transport =
                create_http_transport(url, config.headers.as_ref(), HTTP_CONNECT_TIMEOUT)?;
            McpClient::initialize(config, transport).await
        }
        "stdio" | "" => {
            let transport = spawn_transport(config)?;
            McpClient::initialize(config, transport).await
        }
        "ws" | "websocket" => {
            let url = config.url.as_deref().ok_or_else(|| {
                McpError::Transport(format!(
                    "server '{}' has transport=ws but no url configured",
                    config.name
                ))
            })?;
            let ws_config = WsConfig {
                url: url.to_string(),
                headers: config.headers.clone().unwrap_or_default(),
                headers_helper: None,
            };
            let ws_transport = WebSocketTransport::new(ws_config, WsReconnectConfig::default())?;
            let active = ws_transport.connect().await?;
            let ws_stream = active.into_stream();
            let transport = create_ws_json_rpc_transport(ws_stream);
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

/// Wrap a raw `WebSocketStream` into a `(Sink, Stream)` pair that speaks
/// `JsonRpcMessage`, suitable for passing to `McpClient::initialize()`.
///
/// - **Outgoing**: serializes `JsonRpcMessage` to JSON and sends as a text frame.
/// - **Incoming**: deserializes text frames into `JsonRpcMessage`, ignoring
///   non-text frames (ping/pong/binary/close).
fn create_ws_json_rpc_transport(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> (
    impl futures_util::Sink<
        rmcp::service::TxJsonRpcMessage<rmcp::service::RoleClient>,
        Error = tokio_tungstenite::tungstenite::Error,
    > + Send
    + Unpin
    + 'static,
    impl futures_util::Stream<Item = rmcp::service::RxJsonRpcMessage<rmcp::service::RoleClient>>
    + Send
    + Unpin
    + 'static,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (sink, stream) = ws.split();

    // Map the sink: JsonRpcMessage -> serialize -> Message::Text
    // Uses `with_flat_map` with a sync closure to keep Unpin.
    let mapped_sink = sink.with(
        |msg: rmcp::service::TxJsonRpcMessage<rmcp::service::RoleClient>| {
            let result = serde_json::to_string(&msg)
                .map(|json| Message::Text(json.into()))
                .map_err(|e| {
                    tokio_tungstenite::tungstenite::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("ws json serialize: {e}"),
                    ))
                });
            futures_util::future::ready(result)
        },
    );

    // Map the stream: Message -> extract text -> deserialize -> JsonRpcMessage
    let mapped_stream = stream.filter_map(|result| {
        futures_util::future::ready(match result {
            Ok(Message::Text(text)) => {
                match serde_json::from_str::<
                    rmcp::service::RxJsonRpcMessage<rmcp::service::RoleClient>,
                >(&text)
                {
                    Ok(msg) => Some(msg),
                    Err(e) => {
                        tracing::warn!("ws: failed to parse JSON-RPC message: {e}");
                        None
                    }
                }
            }
            Ok(_) => None, // ignore ping/pong/binary/close
            Err(e) => {
                tracing::warn!("ws: stream error: {e}");
                None
            }
        })
    });

    (mapped_sink, mapped_stream)
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

    /// build_mcp_tools on an empty manager returns an empty Vec.
    #[tokio::test]
    async fn build_mcp_tools_empty_manager_returns_empty() {
        let mgr = McpServerManager::new();
        let tools = mgr.build_mcp_tools().await;
        assert!(tools.is_empty(), "expected no tools from empty manager");
    }

    /// test_disable_enable_server — disable adds to set, enable removes it.
    #[tokio::test]
    async fn test_disable_enable_server() {
        let mgr = McpServerManager::new();

        // Disable a server that doesn't exist in the servers map yet — disabled_names
        // tracks names independently so this should still work.
        mgr.disable_server("my-server")
            .await
            .expect("disable should succeed");

        // Check it's marked disabled
        let info = mgr.get_server_info().await;
        let entry = info.iter().find(|(n, _, _)| n == "my-server");
        assert!(
            entry.is_some(),
            "disabled server should appear in get_server_info"
        );
        let (_, _, disabled) = entry.unwrap();
        assert!(
            *disabled,
            "server should be marked disabled after disable_server()"
        );

        // Enable it
        mgr.enable_server("my-server")
            .await
            .expect("enable should succeed");

        // Now it should not be in the disabled set
        let info2 = mgr.get_server_info().await;
        let entry2 = info2.iter().find(|(n, _, _)| n == "my-server");
        // After enable, if it wasn't in servers map it may not appear, but it must
        // not be disabled. If it does appear, disabled must be false.
        if let Some((_, _, d)) = entry2 {
            assert!(!d, "server should not be disabled after enable_server()");
        }
    }

    /// test_get_server_info_includes_disabled_flag — get_server_info returns
    /// the correct disabled flag after disabling a known (crashed) server.
    #[tokio::test]
    async fn test_get_server_info_includes_disabled_flag() {
        let mgr = McpServerManager::new();

        // Start a server so it appears in the servers map (it will crash)
        let config = ServerConfig {
            name: "info-test-server".into(),
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
        };
        let _ = mgr.start_all(vec![config]).await;

        // Verify it's in the map (crashed)
        let states = mgr.get_server_states().await;
        assert_eq!(states.get("info-test-server"), Some(&ServerState::Crashed));

        // Disable it
        mgr.disable_server("info-test-server")
            .await
            .expect("disable should succeed");

        // get_server_info should show disabled=true
        let info = mgr.get_server_info().await;
        let entry = info.iter().find(|(n, _, _)| n == "info-test-server");
        assert!(entry.is_some(), "server should appear in get_server_info");
        let (_, _, disabled) = entry.unwrap();
        assert!(
            *disabled,
            "get_server_info should return disabled=true after disable_server()"
        );

        // Enable — ignore the transport error (nonexistent binary), the disabled flag is
        // cleared regardless of whether the restart succeeds.
        let _ = mgr.enable_server("info-test-server").await;
        // disabled flag should now be false even if restart fails
        let info3 = mgr.get_server_info().await;
        let entry3 = info3.iter().find(|(n, _, _)| n == "info-test-server");
        assert!(entry3.is_some(), "server should still appear after enable");
        let (_, _, d3) = entry3.unwrap();
        assert!(!d3, "disabled flag should be false after enable_server()");
    }

    /// build_mcp_tools skips servers that are not in Ready state.
    #[tokio::test]
    async fn build_mcp_tools_crashed_server_skipped() {
        let mgr = McpServerManager::new();
        // Start a server that will crash (nonexistent binary)
        let config = ServerConfig {
            name: "crashed-server".into(),
            command: "/nonexistent/binary".into(),
            args: vec![],
            env: HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
        };
        let _ = mgr.start_all(vec![config]).await;

        // Verify it's in Crashed state
        let states = mgr.get_server_states().await;
        assert_eq!(states.get("crashed-server"), Some(&ServerState::Crashed));

        // build_mcp_tools should return empty since no servers are Ready
        let tools = mgr.build_mcp_tools().await;
        assert!(
            tools.is_empty(),
            "crashed server should be skipped; got {} tools",
            tools.len()
        );
    }
}
