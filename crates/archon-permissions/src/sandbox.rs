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
    /// Exact process status when the backend executed a command to completion.
    /// `None` denotes preflight, transport, timeout, or cancellation failure.
    pub exit_code: Option<i32>,
}

/// A request to open a persistent interactive shell (#201 Phase 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxTerminalRequest {
    /// The shell the caller named, or `None` when it did not say. A backend
    /// whose world is Linux must be free to pick its own default: the host
    /// default on Windows is PowerShell, which no Linux image has.
    pub shell: Option<String>,
    /// The session workspace — the root `execute_bash` runs against.
    pub workspace: PathBuf,
    /// Where the shell should start. Equal to `workspace` unless the caller
    /// asked for a subdirectory of it.
    pub cwd: PathBuf,
}

/// Where a terminal opens under a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxTerminal {
    /// This backend does not isolate execution, so a host shell is what being
    /// sandboxed by it already means.
    Host,
    /// Launch this on the host; it is the door into the backend's world, and
    /// the shell comes up on the far side of it.
    Open(SandboxTerminalCommand),
    /// This backend isolates execution and cannot host an interactive shell.
    /// The string says why, in terms the model can act on.
    Refused(String),
}

/// The host command that lands a shell inside a backend's world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxTerminalCommand {
    /// The program spawned on the host — `docker`, `ssh` — not the shell.
    pub program: String,
    pub args: Vec<String>,
    /// The shell that ends up running, by the name the model uses for it.
    pub shell: String,
    /// Where it is running, in the world's own path vocabulary, so the model
    /// is told the paths its commands will actually see.
    pub location: String,
}

/// Backend for sandbox enforcement. The TUI layer implements this and injects
/// it into the tool execution context. Both dispatch paths (main agent direct
/// execute + subagent registry dispatch) consult this before running a tool.
pub trait SandboxBackend: Send + Sync + std::fmt::Debug {
    /// Check whether `tool` with `input` is permitted. Returns `Ok(())` if
    /// allowed, `Err(reason)` if blocked.
    ///
    /// `capability` is the class the tool declares about itself (#201 Phase 3)
    /// and is what a backend decides on. `tool` is carried alongside it only so
    /// a denial can name the call the model made; a backend that branches on
    /// the name has reintroduced the allowlist this replaced.
    fn check(
        &self,
        tool: &str,
        capability: crate::ToolCapability,
        input: &serde_json::Value,
    ) -> Result<(), String>;

    /// Where a persistent terminal opens under this backend.
    ///
    /// Required rather than defaulted, and deliberately so. A default of `Host`
    /// would mean any backend added later hands out an unsandboxed PTY by
    /// omission — which is exactly how terminals came to bypass the isolation
    /// boundary in the first place (#201). A default of `Refused` would be safe
    /// but would silently disable terminals for every policy-only backend. The
    /// answer differs per backend, so every backend states it.
    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal;

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
