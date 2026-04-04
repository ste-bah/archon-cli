use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Permission level -- tools declare their danger level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionLevel {
    Safe,
    Risky,
    Dangerous,
}

// ---------------------------------------------------------------------------
// Agent mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    /// Normal mode -- all tools available.
    Normal,
    /// Plan mode -- only read-only tools allowed.
    Plan,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Normal
    }
}

// ---------------------------------------------------------------------------
// Tool context -- passed to every tool execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub session_id: String,
    pub mode: AgentMode,
}

// ---------------------------------------------------------------------------
// Tool result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        let msg = message.into();
        let content = if msg.starts_with("Error: ") {
            msg
        } else {
            format!("Error: {msg}")
        };
        Self {
            content,
            is_error: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Tool trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used in API tool_use blocks (e.g., "Read", "Write").
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given JSON input.
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;

    /// Classify the permission level for a specific invocation.
    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel;
}
