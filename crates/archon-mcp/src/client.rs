//! MCP client wrapping `rmcp`'s `RunningService<RoleClient>`.
//!
//! Provides a simplified interface for the MCP protocol operations
//! needed by Archon: initialize, list_tools, call_tool, shutdown.

use std::time::Duration;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, ContentBlock,
    ResourceContents, ServerResult,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService, serve_client};
use rmcp::transport::IntoTransport;

use crate::call_cancellation::await_response_cancel_on_drop;
use crate::types::{McpError, McpToolDef, McpToolResult, ServerConfig, ToolContent};

/// Default timeout for the initialize handshake.
const INIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default timeout for tool calls.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// An MCP client connected to a single server process.
pub struct McpClient {
    service: RunningService<RoleClient, ()>,
    server_name: String,
}

impl McpClient {
    /// Connect to an MCP server and perform the initialization handshake.
    ///
    /// Accepts any transport that implements `IntoTransport` (stdio, HTTP, etc.).
    /// Returns an error if the transport fails or the handshake does not
    /// complete within [`INIT_TIMEOUT`].
    pub async fn initialize<T, E, A>(config: &ServerConfig, transport: T) -> Result<Self, McpError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let server_name = config.name.clone();

        let service = tokio::time::timeout(INIT_TIMEOUT, serve_client((), transport))
            .await
            .map_err(|_| McpError::Timeout(INIT_TIMEOUT))?
            .map_err(|e| McpError::InitFailed {
                server: server_name.clone(),
                reason: e.to_string(),
            })?;

        tracing::info!(server = %server_name, "MCP client initialized");

        Ok(Self {
            service,
            server_name,
        })
    }

    /// Retrieve the list of tools advertised by this server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let result = self
            .service
            .list_tools(Default::default())
            .await
            .map_err(|e| {
                McpError::ToolCallFailed(format!(
                    "tools/list failed on '{}': {}",
                    self.server_name, e
                ))
            })?;

        let tools = result
            .tools
            .into_iter()
            .map(|t| {
                let schema = serde_json::Value::Object((*t.input_schema).clone());
                McpToolDef {
                    name: t.name.to_string(),
                    description: t.description.map(|d| d.to_string()),
                    input_schema: schema,
                    annotations: t.annotations,
                    meta: t.meta.and_then(|meta| serde_json::to_value(meta).ok()),
                    server_name: self.server_name.clone(),
                }
            })
            .collect();

        Ok(tools)
    }

    /// Invoke a tool by name with the given JSON arguments.
    ///
    /// Times out after [`CALL_TIMEOUT`].
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<McpToolResult, McpError> {
        let mut params = CallToolRequestParams::default();
        params.name = name.to_string().into();
        params.arguments = arguments;

        // Deliberately not `self.service.call_tool(params)`. That helper hides
        // the request id inside a future which, when dropped, abandons the call
        // silently — and the per-tool budget in `execute_tool_attempt` drops
        // exactly this future. Sending the request ourselves keeps the id in
        // reach so `await_response_cancel_on_drop` can tell the server to stop.
        let handle = self
            .service
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|e| {
                McpError::ToolCallFailed(format!(
                    "tools/call '{}' could not be sent to '{}': {}",
                    name, self.server_name, e
                ))
            })?;

        let response = await_response_cancel_on_drop(handle, CALL_TIMEOUT)
            .await
            .ok_or(McpError::Timeout(CALL_TIMEOUT))?
            .map_err(|e| {
                McpError::ToolCallFailed(format!(
                    "tools/call '{}' failed on '{}': {}",
                    name, self.server_name, e
                ))
            })?;

        let ServerResult::CallToolResult(result) = response else {
            return Err(McpError::ToolCallFailed(format!(
                "tools/call '{}' on '{}' answered with the wrong result type",
                name, self.server_name
            )));
        };

        Ok(convert_tool_result(&result))
    }

    /// Gracefully shut down the connection to the MCP server.
    pub async fn shutdown(self) -> Result<(), McpError> {
        self.service.cancel().await.map_err(|e| {
            McpError::Shutdown(format!("shutdown failed for '{}': {}", self.server_name, e))
        })?;
        Ok(())
    }

    /// The name of the connected server.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}

/// Convert rmcp's `CallToolResult` into our `McpToolResult`.
fn convert_tool_result(result: &CallToolResult) -> McpToolResult {
    let content = result.content.iter().map(convert_content).collect();

    McpToolResult {
        content,
        is_error: result.is_error.unwrap_or(false),
    }
}

/// Convert a single rmcp `ContentBlock` into our `ToolContent`.
fn convert_content(content: &ContentBlock) -> ToolContent {
    match content {
        ContentBlock::Text(t) => ToolContent::Text {
            text: t.text.clone(),
        },
        ContentBlock::Image(img) => ToolContent::Image {
            data: img.data.clone(),
            mime_type: img.mime_type.clone(),
        },
        ContentBlock::Audio(_) => ToolContent::Text {
            text: "[audio content]".into(),
        },
        ContentBlock::Resource(res) => match &res.resource {
            ResourceContents::TextResourceContents { uri, text, .. } => ToolContent::Resource {
                uri: uri.clone(),
                text: Some(text.clone()),
            },
            ResourceContents::BlobResourceContents { uri, .. } => ToolContent::Resource {
                uri: uri.clone(),
                text: None,
            },
            other => unsupported_content("resource", other),
        },
        ContentBlock::ResourceLink(res) => ToolContent::Resource {
            uri: res.uri.clone(),
            text: None,
        },
        other => unsupported_content("content block", other),
    }
}

/// rmcp marks `ContentBlock` and `ResourceContents` `#[non_exhaustive]`, so a
/// future rmcp release can add variants this crate does not know. Pass them on
/// as their JSON wire form rather than dropping the data silently.
fn unsupported_content(kind: &str, value: &impl serde::Serialize) -> ToolContent {
    let text = match serde_json::to_string(value) {
        Ok(json) => format!("[unsupported MCP {kind}] {json}"),
        Err(error) => format!("[unsupported MCP {kind}; could not serialize: {error}]"),
    };
    ToolContent::Text { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::IntoContents;

    /// Helper to build a ContentBlock with text.
    fn text_content(s: &str) -> ContentBlock {
        // Use the IntoContents trait via a string
        let contents: Vec<ContentBlock> = s.to_string().into_contents();
        contents.into_iter().next().expect("at least one content")
    }

    #[test]
    fn convert_text_content() {
        let content = text_content("hello world");
        let converted = convert_content(&content);
        match converted {
            ToolContent::Text { text } => assert_eq!(text, "hello world"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn convert_tool_result_with_error_flag() {
        let mut result = CallToolResult::default();
        result.content = vec![text_content("error message")];
        result.is_error = Some(true);

        let converted = convert_tool_result(&result);
        assert!(converted.is_error);
        assert_eq!(converted.content.len(), 1);
    }

    #[test]
    fn convert_tool_result_no_error() {
        let result = CallToolResult::default();
        let converted = convert_tool_result(&result);
        assert!(!converted.is_error);
        assert!(converted.content.is_empty());
    }

    #[test]
    fn convert_resource_content_text() {
        let resource = ResourceContents::text("file contents", "file:///test.txt");
        let content = ContentBlock::resource(resource);
        let converted = convert_content(&content);
        match converted {
            ToolContent::Resource { uri, text } => {
                assert_eq!(uri, "file:///test.txt");
                assert_eq!(text.unwrap(), "file contents");
            }
            other => panic!("expected Resource, got {other:?}"),
        }
    }

    #[test]
    fn convert_resource_content_blob_has_no_text() {
        let resource = ResourceContents::blob("AAAA", "file:///bin.dat");
        let converted = convert_content(&ContentBlock::resource(resource));
        match converted {
            ToolContent::Resource { uri, text } => {
                assert_eq!(uri, "file:///bin.dat");
                assert!(text.is_none());
            }
            other => panic!("expected Resource, got {other:?}"),
        }
    }

    #[test]
    fn convert_resource_link_keeps_uri() {
        let link = rmcp::model::Resource::new("file:///linked.txt", "linked");
        let converted = convert_content(&ContentBlock::resource_link(link));
        match converted {
            ToolContent::Resource { uri, text } => {
                assert_eq!(uri, "file:///linked.txt");
                assert!(text.is_none());
            }
            other => panic!("expected Resource, got {other:?}"),
        }
    }

    #[test]
    fn convert_image_content() {
        let converted = convert_content(&ContentBlock::image("aGk=", "image/png"));
        match converted {
            ToolContent::Image { data, mime_type } => {
                assert_eq!(data, "aGk=");
                assert_eq!(mime_type, "image/png");
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn initialize_fails_with_bad_transport() {
        let config = ServerConfig {
            name: "test-server".into(),
            command: "echo".into(),
            args: vec![],
            env: std::collections::HashMap::new(),
            disabled: false,
            transport: "stdio".into(),
            url: None,
            headers: None,
            allow_insecure_ws: false,
            tool_policy: Default::default(),
        };
        // echo exits immediately, so initialization should fail
        let transport =
            crate::transport::spawn_transport(&config).expect("spawn should work for echo");
        let result = McpClient::initialize(&config, transport).await;
        assert!(result.is_err());
    }
}
