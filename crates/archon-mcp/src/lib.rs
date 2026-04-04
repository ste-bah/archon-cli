//! archon-mcp: MCP client library for the Archon CLI runtime.
//!
//! Provides .mcp.json config parsing, stdio transport management,
//! MCP protocol client operations, and multi-server lifecycle management.

pub mod types;
pub mod config;
pub mod transport;
pub mod http_transport;
pub mod client;
pub mod lifecycle;
pub mod tool_bridge;
