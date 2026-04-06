use std::sync::Arc;

use archon_tools::mcp_resources::{
    ListMcpResourcesTool, McpResourceContent, McpResourceEntry, ReadMcpResourceTool,
    ResourceProvider,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-session".into(),
        mode: AgentMode::Normal,
            extra_dirs: vec![],
    }
}

// ---------------------------------------------------------------------------
// Mock provider for testing
// ---------------------------------------------------------------------------

struct MockResourceProvider {
    resources: Vec<McpResourceEntry>,
}

#[async_trait::async_trait]
impl ResourceProvider for MockResourceProvider {
    async fn list_resources(
        &self,
        server_filter: Option<&str>,
    ) -> Result<Vec<McpResourceEntry>, String> {
        match server_filter {
            Some(filter) => Ok(self
                .resources
                .iter()
                .filter(|r| r.server == filter)
                .cloned()
                .collect()),
            None => Ok(self.resources.clone()),
        }
    }

    async fn read_resource(&self, uri: &str) -> Result<McpResourceContent, String> {
        // Find in our mock resources
        if self.resources.iter().any(|r| r.uri == uri) {
            Ok(McpResourceContent {
                uri: uri.to_string(),
                mime_type: Some("text/plain".to_string()),
                text: Some(format!("Content of {uri}")),
                blob: None,
                truncated: false,
            })
        } else {
            Err(format!("resource not found: '{uri}'"))
        }
    }
}

fn mock_provider() -> Arc<MockResourceProvider> {
    Arc::new(MockResourceProvider {
        resources: vec![
            McpResourceEntry {
                server: "server-a".into(),
                uri: "file:///config.toml".into(),
                name: "config.toml".into(),
                mime_type: Some("text/plain".into()),
                description: Some("Config file".into()),
            },
            McpResourceEntry {
                server: "server-a".into(),
                uri: "file:///data.json".into(),
                name: "data.json".into(),
                mime_type: Some("application/json".into()),
                description: None,
            },
            McpResourceEntry {
                server: "server-b".into(),
                uri: "db://users".into(),
                name: "users".into(),
                mime_type: None,
                description: Some("User database".into()),
            },
        ],
    })
}

// ---------------------------------------------------------------------------
// Tool name checks
// ---------------------------------------------------------------------------

#[test]
fn list_mcp_resources_tool_name() {
    let tool = ListMcpResourcesTool::default();
    assert_eq!(tool.name(), "ListMcpResources");
}

#[test]
fn read_mcp_resource_tool_name() {
    let tool = ReadMcpResourceTool::default();
    assert_eq!(tool.name(), "ReadMcpResource");
}

// ---------------------------------------------------------------------------
// Schema checks
// ---------------------------------------------------------------------------

#[test]
fn list_has_correct_schema() {
    let tool = ListMcpResourcesTool::default();
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    let props = &schema["properties"];
    assert!(props.get("server").is_some());
    assert_eq!(props["server"]["type"], "string");
    let server_required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().any(|v| v.as_str() == Some("server")))
        .unwrap_or(false);
    assert!(!server_required);
}

#[test]
fn read_has_correct_schema() {
    let tool = ReadMcpResourceTool::default();
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("should have required array");
    assert!(required.iter().any(|v| v.as_str() == Some("uri")));
}

// ---------------------------------------------------------------------------
// ListMcpResources with mock provider
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_all_resources() {
    let provider = mock_provider();
    let tool = ListMcpResourcesTool::new(provider);
    let result = tool.execute(serde_json::json!({}), &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["count"], 3);
    let resources = parsed["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 3);
}

#[tokio::test]
async fn list_resources_filtered_by_server() {
    let provider = mock_provider();
    let tool = ListMcpResourcesTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"server": "server-b"}), &make_ctx())
        .await;
    assert!(!result.is_error);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["count"], 1);
    let resources = parsed["resources"].as_array().unwrap();
    assert_eq!(resources[0]["uri"], "db://users");
}

#[tokio::test]
async fn list_resources_nonexistent_server() {
    let provider = mock_provider();
    let tool = ListMcpResourcesTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"server": "nonexistent"}), &make_ctx())
        .await;
    assert!(!result.is_error);

    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["count"], 0);
}

// ---------------------------------------------------------------------------
// ReadMcpResource with mock provider
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_existing_resource() {
    let provider = mock_provider();
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(
            serde_json::json!({"uri": "file:///config.toml"}),
            &make_ctx(),
        )
        .await;
    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(result.content.contains("Content of file:///config.toml"));
}

#[tokio::test]
async fn read_nonexistent_resource() {
    let provider = mock_provider();
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"uri": "file:///nope.txt"}), &make_ctx())
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("resource not found"));
}

#[tokio::test]
async fn read_missing_uri_returns_error() {
    let tool = ReadMcpResourceTool::default();
    let result = tool.execute(serde_json::json!({}), &make_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("uri"));
}

#[tokio::test]
async fn read_empty_uri_returns_error() {
    let tool = ReadMcpResourceTool::default();
    let result = tool
        .execute(serde_json::json!({"uri": "  "}), &make_ctx())
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("uri"));
}

// ---------------------------------------------------------------------------
// Default (noop) provider
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_list_returns_empty() {
    let tool = ListMcpResourcesTool::default();
    let result = tool.execute(serde_json::json!({}), &make_ctx()).await;
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["count"], 0);
}

#[tokio::test]
async fn default_read_returns_error() {
    let tool = ReadMcpResourceTool::default();
    let result = tool
        .execute(serde_json::json!({"uri": "file:///test"}), &make_ctx())
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("no connected MCP server"));
}

// ---------------------------------------------------------------------------
// Truncation, blob, empty resource tests
// ---------------------------------------------------------------------------

struct LargeContentProvider;

#[async_trait::async_trait]
impl ResourceProvider for LargeContentProvider {
    async fn list_resources(&self, _: Option<&str>) -> Result<Vec<McpResourceEntry>, String> {
        Ok(Vec::new())
    }

    async fn read_resource(&self, uri: &str) -> Result<McpResourceContent, String> {
        match uri {
            "large-ascii" => Ok(McpResourceContent {
                uri: uri.to_string(),
                mime_type: Some("text/plain".into()),
                text: Some("x".repeat(200_000)),
                blob: None,
                truncated: false,
            }),
            "large-utf8" => {
                // Create content with multi-byte UTF-8 chars exceeding 100KB
                let emoji_content = "\u{1F600}".repeat(30_000); // 4 bytes each = 120KB
                Ok(McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("text/plain".into()),
                    text: Some(emoji_content),
                    blob: None,
                    truncated: false,
                })
            }
            "blob-resource" => Ok(McpResourceContent {
                uri: uri.to_string(),
                mime_type: Some("application/octet-stream".into()),
                text: None,
                blob: Some("SGVsbG8gV29ybGQ=".to_string()),
                truncated: false,
            }),
            "empty-resource" => Ok(McpResourceContent {
                uri: uri.to_string(),
                mime_type: None,
                text: None,
                blob: None,
                truncated: false,
            }),
            _ => Err(format!("not found: {uri}")),
        }
    }
}

#[tokio::test]
async fn read_large_ascii_truncated() {
    let provider = Arc::new(LargeContentProvider);
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"uri": "large-ascii"}), &make_ctx())
        .await;
    assert!(!result.is_error);
    assert!(result.content.contains("[Content truncated at 100KB]"));
    // Content should be around 100KB + truncation notice, not 200KB
    assert!(result.content.len() < 110_000);
}

#[tokio::test]
async fn read_large_utf8_no_panic() {
    // This would panic with naive String::truncate if landing mid-char
    let provider = Arc::new(LargeContentProvider);
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"uri": "large-utf8"}), &make_ctx())
        .await;
    assert!(
        !result.is_error,
        "should not panic on multi-byte truncation"
    );
    assert!(result.content.contains("[Content truncated at 100KB]"));
}

#[tokio::test]
async fn read_blob_resource() {
    let provider = Arc::new(LargeContentProvider);
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"uri": "blob-resource"}), &make_ctx())
        .await;
    assert!(!result.is_error);
    assert!(
        result.content.contains("[base64]"),
        "blob should have [base64] prefix, got: {}",
        result.content
    );
    assert!(result.content.contains("SGVsbG8gV29ybGQ="));
}

#[tokio::test]
async fn read_empty_resource() {
    let provider = Arc::new(LargeContentProvider);
    let tool = ReadMcpResourceTool::new(provider);
    let result = tool
        .execute(serde_json::json!({"uri": "empty-resource"}), &make_ctx())
        .await;
    assert!(!result.is_error);
    assert!(result.content.contains("[empty resource]"));
}

