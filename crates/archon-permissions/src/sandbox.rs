//! GHOST-006: SandboxBackend trait for dependency-inverted sandbox enforcement.
//!
//! Lives in archon-permissions (leaf crate) so both archon-tools (ToolContext)
//! and archon-tui (impl) can depend on it without circularity.

/// Backend for sandbox enforcement. The TUI layer implements this and injects
/// it into the tool execution context. Both dispatch paths (main agent direct
/// execute + subagent registry dispatch) consult this before running a tool.
pub trait SandboxBackend: Send + Sync + std::fmt::Debug {
    /// Check whether `tool` with `input` is permitted. Returns `Ok(())` if
    /// allowed, `Err(reason)` if blocked.
    fn check(&self, tool: &str, input: &serde_json::Value) -> Result<(), String>;
}
