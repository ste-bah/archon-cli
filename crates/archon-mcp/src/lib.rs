//! archon-mcp: MCP client library for the Archon CLI runtime.
//!
//! Provides .mcp.json config parsing, stdio transport management,
//! MCP protocol client operations, and multi-server lifecycle management.

pub mod client;
pub mod config;
pub mod http_transport;
pub mod lifecycle;
pub mod tool_bridge;
pub mod transport;
pub mod transport_ws;
pub mod types;
pub mod ws_config;
