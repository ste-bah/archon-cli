//! MCP resource tools: ListMcpResources and ReadMcpResource.
//!
//! These tools wrap the MCP resource protocol, allowing the agent to
//! enumerate and read resources exposed by connected MCP servers.
//!
//! The tools accept a [`ResourceProvider`] trait object that abstracts
//! the actual MCP protocol calls. The `archon-core` crate wires the
//! real implementation backed by `McpServerManager`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// ResourceProvider trait — implemented by archon-core to bridge to MCP
// ---------------------------------------------------------------------------

/// Maximum resource content size returned (100 KB).
const MAX_CONTENT_BYTES: usize = 102_400;

/// A resource entry returned by listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceEntry {
    pub server: String,
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
}

/// Content of a read resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceContent {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
    pub truncated: bool,
}

/// Trait for listing and reading MCP resources. Implemented by the
/// integration layer that has access to `McpServerManager`.
#[async_trait::async_trait]
pub trait ResourceProvider: Send + Sync {
    /// List resources from all connected servers, optionally filtered by server name.
    async fn list_resources(
        &self,
        server_filter: Option<&str>,
    ) -> Result<Vec<McpResourceEntry>, String>;

    /// Read a resource by URI, routing to the correct MCP server.
    async fn read_resource(&self, uri: &str) -> Result<McpResourceContent, String>;
}

/// A no-op provider that reports no resources. Used when no MCP servers are configured.
pub struct NoopResourceProvider;

#[async_trait::async_trait]
impl ResourceProvider for NoopResourceProvider {
    async fn list_resources(
        &self,
        _server_filter: Option<&str>,
    ) -> Result<Vec<McpResourceEntry>, String> {
        Ok(Vec::new())
    }

    async fn read_resource(&self, uri: &str) -> Result<McpResourceContent, String> {
        Err(format!(
            "resource not found: no connected MCP server exposes '{uri}'"
        ))
    }
}

// ---------------------------------------------------------------------------
// ListMcpResources
// ---------------------------------------------------------------------------

/// Lists resources available from connected MCP servers.
pub struct ListMcpResourcesTool {
    provider: Arc<dyn ResourceProvider>,
}

impl ListMcpResourcesTool {
    pub fn new(provider: Arc<dyn ResourceProvider>) -> Self {
        Self { provider }
    }
}

impl Default for ListMcpResourcesTool {
    fn default() -> Self {
        Self {
            provider: Arc::new(NoopResourceProvider),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResources"
    }

    fn description(&self) -> &str {
        "List resources available from connected MCP servers. Optionally filter by server name."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Optional server name to filter resources by."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let server_filter = input.get("server").and_then(|v| v.as_str());

        match self.provider.list_resources(server_filter).await {
            Ok(resources) => {
                let result = json!({
                    "resources": resources,
                    "count": resources.len(),
                });
                match serde_json::to_string_pretty(&result) {
                    Ok(s) => ToolResult::success(s),
                    Err(e) => ToolResult::error(format!("failed to serialize resource list: {e}")),
                }
            }
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// ReadMcpResource
// ---------------------------------------------------------------------------

/// Reads the content of a specific MCP resource by URI.
pub struct ReadMcpResourceTool {
    provider: Arc<dyn ResourceProvider>,
}

impl ReadMcpResourceTool {
    pub fn new(provider: Arc<dyn ResourceProvider>) -> Self {
        Self { provider }
    }
}

impl Default for ReadMcpResourceTool {
    fn default() -> Self {
        Self {
            provider: Arc::new(NoopResourceProvider),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "ReadMcpResource"
    }

    fn description(&self) -> &str {
        "Read the content of an MCP resource by its URI. Text resources are returned \
         as plain text, binary resources as base64. Content truncated at 100KB."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The URI of the MCP resource to read."
                }
            },
            "required": ["uri"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let uri = match input.get("uri").and_then(|v| v.as_str()) {
            Some(u) if !u.trim().is_empty() => u,
            _ => return ToolResult::error("missing required parameter 'uri'"),
        };

        match self.provider.read_resource(uri).await {
            Ok(content) => {
                // Return text content directly, blob as base64 string
                let display = if let Some(ref text) = content.text {
                    let mut t = text.clone();
                    if t.len() > MAX_CONTENT_BYTES {
                        // Safe truncation respecting UTF-8 char boundaries
                        let mut truncate_at = MAX_CONTENT_BYTES;
                        while !t.is_char_boundary(truncate_at) {
                            truncate_at -= 1;
                        }
                        t.truncate(truncate_at);
                        t.push_str("\n\n[Content truncated at 100KB]");
                    }
                    t
                } else if let Some(ref blob) = content.blob {
                    let mut b = blob.clone();
                    if b.len() > MAX_CONTENT_BYTES {
                        let mut truncate_at = MAX_CONTENT_BYTES;
                        while !b.is_char_boundary(truncate_at) {
                            truncate_at -= 1;
                        }
                        b.truncate(truncate_at);
                        b.push_str("\n\n[Content truncated at 100KB]");
                    }
                    format!("[base64] {b}")
                } else {
                    "[empty resource]".to_string()
                };

                ToolResult::success(display)
            }
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tool_name() {
        assert_eq!(ListMcpResourcesTool::default().name(), "ListMcpResources");
    }

    #[test]
    fn read_tool_name() {
        assert_eq!(ReadMcpResourceTool::default().name(), "ReadMcpResource");
    }

    #[test]
    fn list_schema_has_server_property() {
        let schema = ListMcpResourcesTool::default().input_schema();
        assert!(schema["properties"]["server"].is_object());
    }

    #[test]
    fn read_schema_requires_uri() {
        let schema = ReadMcpResourceTool::default().input_schema();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|v| v == "uri"));
    }
}
