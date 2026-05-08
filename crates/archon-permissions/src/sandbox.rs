//! GHOST-006: SandboxBackend trait for dependency-inverted sandbox enforcement.
//!
//! Lives in archon-permissions (leaf crate) so both archon-tools (ToolContext)
//! and archon-tui (impl) can depend on it without circularity.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommandRequest {
    pub command: String,
    pub working_dir: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommandResult {
    pub content: String,
    pub is_error: bool,
}

/// Backend for sandbox enforcement. The TUI layer implements this and injects
/// it into the tool execution context. Both dispatch paths (main agent direct
/// execute + subagent registry dispatch) consult this before running a tool.
pub trait SandboxBackend: Send + Sync + std::fmt::Debug {
    /// Check whether `tool` with `input` is permitted. Returns `Ok(())` if
    /// allowed, `Err(reason)` if blocked.
    fn check(&self, tool: &str, input: &serde_json::Value) -> Result<(), String>;

    /// Optionally execute Bash inside this backend. Logical policy-only
    /// backends return `None`, which tells the tool to use the normal host
    /// execution path after `check` has allowed it.
    fn execute_bash<'a>(
        &'a self,
        _request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        Box::pin(async { None })
    }
}
