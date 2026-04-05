//! JSON-RPC 2.0 protocol types for IDE extension communication (TASK-CLI-411).
//!
//! Implements the full set of request/response/notification types for the
//! archon/initialize, archon/prompt, archon/cancel, archon/toolResult,
//! archon/status, and archon/config methods, plus all server→client
//! notification types.

use serde::{Deserialize, Serialize};

// ── Core JSON-RPC 2.0 framing types ──────────────────────────────────────────

/// A JSON-RPC 2.0 request (client → server, expects a response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JRpcRequest {
    /// Must always be "2.0".
    pub jsonrpc: String,
    /// Request identifier; matched in the response.
    pub id: u64,
    /// Method name, e.g. `"archon/initialize"`.
    pub method: String,
    /// Method-specific parameters.
    pub params: serde_json::Value,
}

/// A JSON-RPC 2.0 response (server → client, in reply to a request).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JRpcResponse {
    /// Must always be "2.0".
    pub jsonrpc: String,
    /// Matches the `id` from the request.
    pub id: u64,
    /// Successful result, present when `error` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error object, present when `result` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JRpcError>,
}

/// A JSON-RPC 2.0 notification (server → client, no response expected, no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JRpcNotification {
    /// Must always be "2.0".
    pub jsonrpc: String,
    /// Notification method name, e.g. `"archon/textDelta"`.
    pub method: String,
    /// Notification parameters.
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 error object embedded in a [`JRpcResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JRpcError {
    /// Numeric error code (see [`JRpcErrorCode`]).
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional additional error data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Standard JSON-RPC 2.0 error codes.
pub struct JRpcErrorCode;

impl JRpcErrorCode {
    /// Parse error — invalid JSON received.
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid Request — the JSON is not a valid Request object.
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found — the method does not exist.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params — invalid method parameter(s).
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error — internal JSON-RPC error.
    pub const INTERNAL_ERROR: i32 = -32603;
}

// ── Internal session tracking type ───────────────────────────────────────────

/// An active IDE session tracked by [`crate::ide::handler::IdeProtocolHandler`].
#[derive(Debug, Clone)]
pub struct IdeSession {
    /// Unique session identifier.
    pub session_id: String,
    /// Capabilities negotiated during `archon/initialize`.
    pub capabilities: IdeCapabilities,
}

// ── IDE-specific parameter and result types ───────────────────────────────────

/// Capabilities advertised by the IDE client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeCapabilities {
    /// Client supports inline completion rendering.
    pub inline_completion: bool,
    /// Client supports tool execution approval UI.
    pub tool_execution: bool,
    /// Client supports diff view rendering.
    pub diff: bool,
    /// Client supports terminal integration.
    pub terminal: bool,
}

/// Identifying information about the IDE client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeClientInfo {
    /// Extension or client name (e.g. `"vscode-archon"`).
    pub name: String,
    /// Extension or client version (semver string).
    pub version: String,
}

/// Parameters for `archon/initialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeInitializeParams {
    /// Identifying information about the IDE client.
    pub client_info: IdeClientInfo,
    /// Capabilities the client supports.
    pub capabilities: IdeCapabilities,
}

/// Result for `archon/initialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeInitializeResult {
    /// Unique session ID assigned by the server.
    pub session_id: String,
    /// Server version string.
    pub server_version: String,
    /// Capabilities the server supports.
    pub capabilities: IdeCapabilities,
}

/// Parameters for `archon/prompt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdePromptParams {
    /// Session ID from [`IdeInitializeResult`].
    pub session_id: String,
    /// The user's prompt text.
    pub text: String,
    /// Optional list of file paths to include as context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_files: Option<Vec<String>>,
}

/// Parameters for `archon/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeCancelParams {
    /// Session ID to cancel.
    pub session_id: String,
}

/// Parameters for `archon/toolResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeToolResultParams {
    /// Session ID.
    pub session_id: String,
    /// ID of the tool use being answered.
    pub tool_use_id: String,
    /// Result content string.
    pub result: String,
    /// Whether this result represents an error.
    pub is_error: bool,
}

/// Parameters for `archon/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeStatusParams {
    /// Session ID to query.
    pub session_id: String,
}

/// Result for `archon/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeStatusResult {
    /// Model name in use for this session.
    pub model: String,
    /// Total input tokens consumed in this session.
    pub input_tokens: u64,
    /// Total output tokens consumed in this session.
    pub output_tokens: u64,
    /// Estimated cost in USD for this session.
    pub cost: f64,
}

/// Parameters for `archon/config`.
///
/// If `key` is present with no `value`, this is a read operation.
/// If both `key` and `value` are present, this is a write operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeConfigParams {
    /// Configuration key to read or write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Value to set; absent means read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

// ── Server → client notification payload types ────────────────────────────────

/// Payload for `archon/textDelta` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeTextDelta {
    /// Session that produced this delta.
    pub session_id: String,
    /// Incremental text to append.
    pub text: String,
}

/// Payload for `archon/thinkingDelta` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeThinkingDelta {
    /// Session that produced this thinking delta.
    pub session_id: String,
    /// Incremental thinking text.
    pub thinking: String,
}

/// Payload for `archon/toolCall` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeToolCall {
    /// Session requesting tool execution.
    pub session_id: String,
    /// Unique identifier for this tool invocation.
    pub tool_use_id: String,
    /// Tool name.
    pub name: String,
    /// Tool input parameters as a JSON value.
    pub input: serde_json::Value,
}

/// Payload for `archon/permissionRequest` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdePermissionRequest {
    /// Session requesting permission.
    pub session_id: String,
    /// The action requiring approval (e.g. `"write_file"`).
    pub action: String,
    /// Human-readable description of what the agent wants to do.
    pub description: String,
}

/// Payload for `archon/turnComplete` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeTurnComplete {
    /// Session that completed the turn.
    pub session_id: String,
    /// Input tokens consumed in this turn.
    pub input_tokens: u64,
    /// Output tokens consumed in this turn.
    pub output_tokens: u64,
    /// Estimated cost in USD for this turn.
    pub cost: f64,
}

/// Payload for `archon/error` notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeError {
    /// Session ID, if the error is session-scoped; `None` for global errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Error message.
    pub message: String,
    /// Error code (use [`JRpcErrorCode`] constants).
    pub code: i32,
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Parse a raw JSON-RPC request string and extract `(id, method, params)`.
///
/// Returns an error if the JSON is malformed, or if required fields are absent.
pub fn parse_request(json: &str) -> anyhow::Result<(u64, String, serde_json::Value)> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow::anyhow!("parse error: {e}"))?;

    let id = v
        .get("id")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing or non-integer 'id' field"))?;

    let method = v
        .get("method")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'method' field"))?
        .to_string();

    let params = v.get("params").cloned().unwrap_or(serde_json::Value::Null);

    Ok((id, method, params))
}

/// Serialize a JSON-RPC error response to a JSON string.
///
/// Produces: `{"jsonrpc":"2.0","id":<id>,"error":{"code":<code>,"message":<msg>}}`
pub fn error_response(id: u64, code: i32, message: &str) -> String {
    let resp = JRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":"internal serialization error"}}}}"#
        )
    })
}

/// Build a successful JSON-RPC response string.
pub fn success_response(id: u64, result: serde_json::Value) -> String {
    let resp = JRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    };
    serde_json::to_string(&resp)
        .unwrap_or_else(|_| format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#))
}
