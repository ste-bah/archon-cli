//! In-process MCP server for tool registration (TASK-CLI-305).
//!
//! Provides `SdkTool` and `SdkMcpServer` so consumers can register custom tools
//! that the agent can call during a `query()` or session `send()`.

use std::sync::Arc;

use crate::error::SdkError;

/// The async handler function type for an `SdkTool`.
///
/// Receives the tool input as a JSON value and returns a string result.
pub type ToolHandler = Arc<
    dyn Fn(
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, SdkError>> + Send + 'static>,
        > + Send
        + Sync,
>;

/// A tool definition for use with [`SdkMcpServer`].
pub struct SdkTool {
    /// Tool name used in API requests and tool-use blocks.
    pub name: String,
    /// Human-readable description shown to the model.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub schema: serde_json::Value,
    /// Async handler invoked when the tool is called.
    pub handler: ToolHandler,
}

impl SdkTool {
    /// Produce the Anthropic-format tool definition used in API requests.
    ///
    /// ```json
    /// { "name": "...", "description": "...", "input_schema": { ... } }
    /// ```
    pub fn to_api_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.schema,
        })
    }

    /// Invoke the tool handler with the given input.
    pub async fn call(&self, input: serde_json::Value) -> Result<String, SdkError> {
        (self.handler)(input).await
    }
}

impl std::fmt::Debug for SdkTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

/// In-process MCP server holding a named collection of [`SdkTool`]s.
///
/// Create one via [`create_sdk_mcp_server`] and pass it in [`crate::SdkOptions`].
#[derive(Debug)]
pub struct SdkMcpServer {
    /// Server name (informational).
    name: String,
    tools: Vec<SdkTool>,
}

impl SdkMcpServer {
    /// Server name as supplied to [`create_sdk_mcp_server`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return Anthropic-format tool schemas for all registered tools.
    pub fn tool_schemas(&self) -> Vec<serde_json::Value> {
        self.tools.iter().map(|t| t.to_api_schema()).collect()
    }

    /// Find a tool by name and invoke it with the given input.
    pub async fn dispatch(
        &self,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<String, SdkError> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == tool_name)
            .ok_or_else(|| SdkError::Tool(format!("unknown tool: {tool_name}")))?;
        tool.call(input).await
    }
}

/// Create an in-process MCP server with the given name and tool list.
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use archon_sdk::{create_sdk_mcp_server, SdkTool};
///
/// let tool = SdkTool {
///     name: "echo".into(),
///     description: "Echo the input".into(),
///     schema: serde_json::json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
///     handler: Arc::new(|input| Box::pin(async move {
///         Ok(input["text"].as_str().unwrap_or("").to_string())
///     })),
/// };
/// let server = create_sdk_mcp_server("my-server", vec![tool]);
/// ```
pub fn create_sdk_mcp_server(name: impl Into<String>, tools: Vec<SdkTool>) -> SdkMcpServer {
    SdkMcpServer {
        name: name.into(),
        tools,
    }
}
