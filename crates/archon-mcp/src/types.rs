//! MCP type definitions for server configuration, state, tool metadata, and errors.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Permission policy Archon should apply to a bridged MCP tool.
///
/// Unknown tools intentionally default to Risky at the bridge layer. This enum
/// exists for explicit operator config, not for trusting arbitrary server text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum McpToolRisk {
    Safe,
    Risky,
    Dangerous,
}

impl McpToolRisk {
    pub fn as_permission_level(self) -> archon_tools::tool::PermissionLevel {
        match self {
            Self::Safe => archon_tools::tool::PermissionLevel::Safe,
            Self::Risky => archon_tools::tool::PermissionLevel::Risky,
            Self::Dangerous => archon_tools::tool::PermissionLevel::Dangerous,
        }
    }
}

/// Operator policy for bridging tools from a single MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolBridgePolicy {
    /// Whether to trust this server's MCP annotations and `_meta` permission
    /// hints. Defaults false because MCP annotations are advisory.
    #[serde(default)]
    pub trust_server_hints: bool,
    /// Explicit per-tool permission overrides keyed by raw tool name or by
    /// Archon's qualified `mcp__server__tool` name.
    #[serde(default)]
    pub tool_permissions: HashMap<String, McpToolRisk>,
}

/// Configuration for a single MCP server, parsed from .mcp.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Human-readable server name (the JSON key).
    pub name: String,
    /// Command to execute (e.g. "npx", "python3"). Empty for HTTP transport.
    #[serde(default)]
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables injected into the child process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// When true the server is skipped during startup.
    #[serde(default)]
    pub disabled: bool,
    /// Transport type: "stdio" (default), "http", "ws"/"websocket", or "sse".
    #[serde(default = "default_transport")]
    pub transport: String,
    /// Endpoint URL required by network transports.
    #[serde(default)]
    pub url: Option<String>,
    /// Custom headers (e.g. Authorization) for network transports.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Explicitly allow non-loopback plaintext WebSocket MCP endpoints.
    ///
    /// Defaults false. Prefer `wss://` for any remote endpoint.
    #[serde(default, rename = "allowInsecureWs", alias = "allow_insecure_ws")]
    pub allow_insecure_ws: bool,
    /// Tool bridge permission policy for this server.
    #[serde(default, rename = "toolPolicy", alias = "tool_policy")]
    pub tool_policy: McpToolBridgePolicy,
}

fn default_transport() -> String {
    "stdio".into()
}

/// Runtime state of a managed MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerState {
    /// Server process is being spawned and initialized.
    Starting,
    /// Initialization handshake completed successfully.
    Ready,
    /// Server process exited unexpectedly.
    Crashed,
    /// Automatic restart is in progress.
    Restarting,
    /// Server has been intentionally or permanently stopped.
    Stopped,
}

impl std::fmt::Display for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Ready => write!(f, "ready"),
            Self::Crashed => write!(f, "crashed"),
            Self::Restarting => write!(f, "restarting"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// All errors originating from the archon-mcp crate.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("config parse error: {0}")]
    ConfigParse(String),

    #[error("config I/O error: {0}")]
    ConfigIo(#[from] std::io::Error),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("initialization failed for server '{server}': {reason}")]
    InitFailed { server: String, reason: String },

    #[error("tool call failed: {0}")]
    ToolCallFailed(String),

    #[error("server '{0}' not found")]
    ServerNotFound(String),

    #[error("server '{0}' is not ready (state: {1})")]
    ServerNotReady(String, ServerState),

    #[error("operation timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("shutdown error: {0}")]
    Shutdown(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("max restarts exceeded for server '{0}'")]
    MaxRestartsExceeded(String),
}

/// A tool definition advertised by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    /// Fully qualified tool name.
    pub name: String,
    /// Human-readable description (may be absent).
    pub description: Option<String>,
    /// JSON Schema describing expected input arguments.
    pub input_schema: serde_json::Value,
    /// Optional MCP tool annotations. These are advisory and must only affect
    /// permissions when the server is explicitly trusted.
    #[serde(default)]
    pub annotations: Option<rmcp::model::ToolAnnotations>,
    /// Optional MCP `_meta` payload, preserved for explicit Archon permission
    /// hints from trusted servers.
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
    /// Which MCP server provides this tool.
    pub server_name: String,
}

/// Result returned from a tools/call invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Serialised content returned by the tool.
    pub content: Vec<ToolContent>,
    /// True when the tool reported an error.
    pub is_error: bool,
}

/// A single piece of content inside a tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { uri: String, text: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_state_display() {
        assert_eq!(ServerState::Starting.to_string(), "starting");
        assert_eq!(ServerState::Ready.to_string(), "ready");
        assert_eq!(ServerState::Crashed.to_string(), "crashed");
        assert_eq!(ServerState::Restarting.to_string(), "restarting");
        assert_eq!(ServerState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn server_config_deserialize_minimal() {
        let json = r#"{"name":"test","command":"echo","args":[]}"#;
        let cfg: ServerConfig = serde_json::from_str(json).expect("parse");
        assert_eq!(cfg.name, "test");
        assert_eq!(cfg.command, "echo");
        assert!(!cfg.disabled);
        assert!(cfg.env.is_empty());
        assert_eq!(cfg.transport, "stdio");
        assert!(cfg.url.is_none());
        assert!(cfg.headers.is_none());
        assert!(!cfg.tool_policy.trust_server_hints);
    }

    #[test]
    fn server_config_deserialize_full() {
        let json = r#"{
            "name": "my-server",
            "command": "node",
            "args": ["server.js", "--port", "3000"],
            "env": {"NODE_ENV": "production"},
            "disabled": true
        }"#;
        let cfg: ServerConfig = serde_json::from_str(json).expect("parse");
        assert_eq!(cfg.name, "my-server");
        assert_eq!(cfg.args, vec!["server.js", "--port", "3000"]);
        assert_eq!(cfg.env.get("NODE_ENV").unwrap(), "production");
        assert!(cfg.disabled);
        assert_eq!(cfg.transport, "stdio");
    }

    #[test]
    fn server_config_deserialize_http() {
        let json = r#"{
            "name": "remote",
            "command": "",
            "transport": "http",
            "url": "http://localhost:8080/mcp",
            "headers": {"Authorization": "Bearer tok123"}
        }"#;
        let cfg: ServerConfig = serde_json::from_str(json).expect("parse");
        assert_eq!(cfg.transport, "http");
        assert_eq!(cfg.url.as_deref(), Some("http://localhost:8080/mcp"));
        let headers = cfg.headers.expect("headers");
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer tok123");
    }

    #[test]
    fn server_config_deserialize_tool_policy() {
        let json = r#"{
            "name": "trusted-local",
            "transport": "stdio",
            "toolPolicy": {
                "trustServerHints": true,
                "toolPermissions": {
                    "delete_repo": "dangerous",
                    "mcp__trusted-local__search": "safe"
                }
            }
        }"#;
        let cfg: ServerConfig = serde_json::from_str(json).expect("parse");
        assert!(cfg.tool_policy.trust_server_hints);
        assert_eq!(
            cfg.tool_policy.tool_permissions.get("delete_repo"),
            Some(&McpToolRisk::Dangerous)
        );
        assert_eq!(
            cfg.tool_policy
                .tool_permissions
                .get("mcp__trusted-local__search"),
            Some(&McpToolRisk::Safe)
        );
    }

    #[test]
    fn mcp_error_display() {
        let err = McpError::ServerNotFound("foo".into());
        assert_eq!(err.to_string(), "server 'foo' not found");

        let err = McpError::ServerNotReady("bar".into(), ServerState::Crashed);
        assert_eq!(
            err.to_string(),
            "server 'bar' is not ready (state: crashed)"
        );
    }

    #[test]
    fn mcp_tool_def_roundtrip() {
        let tool = McpToolDef {
            name: "read_file".into(),
            description: Some("Read a file from disk".into()),
            input_schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            annotations: Some(rmcp::model::ToolAnnotations::new().read_only(true)),
            meta: Some(serde_json::json!({"archon": {"permissionLevel": "safe"}})),
            server_name: "filesystem".into(),
        };
        let json = serde_json::to_string(&tool).expect("serialize");
        let parsed: McpToolDef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.name, "read_file");
        assert_eq!(parsed.server_name, "filesystem");
        assert_eq!(
            parsed.annotations.and_then(|a| a.read_only_hint),
            Some(true)
        );
        assert_eq!(
            parsed
                .meta
                .and_then(|m| m.pointer("/archon/permissionLevel").cloned()),
            Some(serde_json::json!("safe"))
        );
    }

    #[test]
    fn mcp_tool_result_with_error() {
        let result = McpToolResult {
            content: vec![ToolContent::Text {
                text: "something went wrong".into(),
            }],
            is_error: true,
        };
        assert!(result.is_error);
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn tool_content_serde_text() {
        let content = ToolContent::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&content).expect("serialize");
        assert!(json.contains(r#""type":"text"#));
        let parsed: ToolContent = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            ToolContent::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text variant"),
        }
    }
}
