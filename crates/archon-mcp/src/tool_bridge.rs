//! Bridge between MCP tool definitions and the Archon `Tool` trait.
//!
//! `McpTool` wraps a remote MCP tool so the LLM can discover and invoke it
//! through the same interface as built-in tools. All MCP tools are prefixed
//! `mcp__{server_name}__{tool_name}` and classified as [`PermissionLevel::Risky`].

use std::sync::Arc;

use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

use crate::client::McpClient;
use crate::types::{McpToolDef, ToolContent};

/// Build the fully qualified tool name for an MCP tool.
pub fn qualified_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp__{server_name}__{tool_name}")
}

/// An MCP-backed tool that proxies execution to a remote MCP server.
pub struct McpTool {
    /// Fully qualified name: `mcp__{server}__{tool}`.
    qualified_name: String,
    /// The original tool definition from the server.
    tool_def: McpToolDef,
    /// Shared client used for RPC calls.
    client: Arc<McpClient>,
}

impl McpTool {
    /// Create a new bridge tool.
    ///
    /// The tool name is constructed as `mcp__{server_name}__{tool_name}`.
    pub fn new(server_name: &str, tool_def: McpToolDef, client: Arc<McpClient>) -> Self {
        let qualified_name = qualified_tool_name(server_name, &tool_def.name);
        Self {
            qualified_name,
            tool_def,
            client,
        }
    }

}

/// Flatten MCP tool content into a single text string.
fn flatten_content(content: &[ToolContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text { text } => Some(text.as_str()),
            ToolContent::Resource { text, .. } => text.as_deref(),
            ToolContent::Image { .. } => Some("[image content]"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        self.tool_def
            .description
            .as_deref()
            .unwrap_or("MCP tool (no description)")
    }

    fn input_schema(&self) -> serde_json::Value {
        self.tool_def.input_schema.clone()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Convert Value to a Map for call_tool.
        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return ToolResult::error(format!(
                    "MCP tool '{}' expects an object input, got: {}",
                    self.qualified_name, other
                ));
            }
        };

        match self.client.call_tool(&self.tool_def.name, arguments).await {
            Ok(mcp_result) => {
                let text = flatten_content(&mcp_result.content);
                if mcp_result.is_error {
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Err(e) => {
                ToolResult::error(format!("MCP call to '{}' failed: {e}", self.qualified_name))
            }
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::McpToolResult;

    #[test]
    fn qualified_name_construction() {
        assert_eq!(
            qualified_tool_name("memorygraph", "store_memory"),
            "mcp__memorygraph__store_memory"
        );
        assert_eq!(
            qualified_tool_name("serena", "find_symbol"),
            "mcp__serena__find_symbol"
        );
    }

    #[test]
    fn qualified_name_with_special_chars() {
        assert_eq!(
            qualified_tool_name("my-server", "do_thing"),
            "mcp__my-server__do_thing"
        );
    }

    #[test]
    fn flatten_content_text_only() {
        let content = vec![
            ToolContent::Text {
                text: "line 1".into(),
            },
            ToolContent::Text {
                text: "line 2".into(),
            },
        ];
        assert_eq!(flatten_content(&content), "line 1\nline 2");
    }

    #[test]
    fn flatten_content_mixed() {
        let content = vec![
            ToolContent::Text {
                text: "hello".into(),
            },
            ToolContent::Image {
                data: "base64data".into(),
                mime_type: "image/png".into(),
            },
            ToolContent::Resource {
                uri: "file:///test".into(),
                text: Some("resource text".into()),
            },
        ];
        assert_eq!(
            flatten_content(&content),
            "hello\n[image content]\nresource text"
        );
    }

    #[test]
    fn flatten_content_empty() {
        assert_eq!(flatten_content(&[]), "");
    }

    #[test]
    fn flatten_content_resource_no_text() {
        let content = vec![ToolContent::Resource {
            uri: "file:///bin".into(),
            text: None,
        }];
        // Resource with no text is filtered out.
        assert_eq!(flatten_content(&content), "");
    }

    #[test]
    fn flatten_error_result() {
        let result = McpToolResult {
            content: vec![ToolContent::Text {
                text: "something broke".into(),
            }],
            is_error: true,
        };
        let text = flatten_content(&result.content);
        assert_eq!(text, "something broke");
        assert!(result.is_error);
    }

    // NOTE: McpTool::new() and the Tool trait methods (name, description,
    // input_schema, permission_level, execute) require an Arc<McpClient>,
    // which needs a live MCP server connection. Those are covered by
    // integration tests. The unit tests above verify the helper functions
    // that McpTool delegates to.
}
