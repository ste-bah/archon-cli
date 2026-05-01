use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Normal mode -- all tools available.
    #[default]
    Normal,
    /// Plan mode -- only read-only tools allowed.
    Plan,
}

// ---------------------------------------------------------------------------
// Tool context -- passed to every tool execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub session_id: String,
    pub mode: AgentMode,
    /// Additional directories added at runtime via `/add-dir`.
    pub extra_dirs: Vec<PathBuf>,
    /// TASK-AGS-105: true if the parent agent is currently inside a fork
    /// child (computed via `is_in_fork_child_by_messages` at turn start
    /// on the Agent side). Used by `SubagentExecutor` implementations
    /// to block fork-in-fork without crossing the `state.messages`
    /// boundary into archon-tools. Default is `false` for all non-Agent
    /// construction sites.
    pub in_fork: bool,
    /// TASK-AGS-105: true if this tool invocation was routed via
    /// `TaskCreate` (as opposed to the direct `Agent` tool). Preserves
    /// the `nested: bool` argument semantics from the old
    /// `Agent::handle_subagent_result(tool_result, nested)` helper:
    /// when `nested == true`, the executor fires the `TaskCompleted`
    /// hook on successful completion. Name retained verbatim — do NOT
    /// rename to `is_nested`, `spawned_from_task_create`, etc.
    pub nested: bool,
    /// TASK-AGS-107: parent CancellationToken for cascading cancellation.
    /// When set, `AgentTool::execute` creates a `child_token()` so that
    /// cancelling the parent (e.g. Ctrl+C in the input handler) cascades
    /// to all spawned subagents. `None` for top-level tool invocations
    /// where no parent cancel exists.
    pub cancel_parent: Option<CancellationToken>,
    /// GHOST-006: sandbox enforcement backend. When set, both dispatch paths
    /// (agent.rs direct execute + dispatch.rs subagent path) check this
    /// before running a tool. Toggled via `/sandbox on/off`.
    pub sandbox: Option<std::sync::Arc<dyn archon_permissions::SandboxBackend>>,
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
