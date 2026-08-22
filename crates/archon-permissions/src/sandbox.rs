//! GHOST-006: SandboxBackend trait for dependency-inverted sandbox enforcement.
//!
//! Lives in archon-permissions (leaf crate) so both archon-tools (ToolContext)
//! and archon-tui (impl) can depend on it without circularity.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

/// How long a sandbox lives before it is destroyed (`sandbox.scope`).
///
/// The knob shipped validated, audited and printed, and read by nothing, so a
/// `docker` session built and destroyed a container per command whatever it was
/// set to. Every build cache a command warmed — `~/.cargo/registry`, `~/.npm`,
/// pip wheels, apt lists — died with that container, because the bind mount
/// covers only the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SandboxScope {
    /// One sandbox for the whole run.
    #[default]
    Session,
    /// A fresh sandbox per agent turn.
    Turn,
    /// A fresh sandbox per command.
    Tool,
}

impl SandboxScope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Turn => "turn",
            Self::Tool => "tool",
        }
    }
}

impl std::fmt::Display for SandboxScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SandboxScope {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "session" => Ok(Self::Session),
            "turn" => Ok(Self::Turn),
            "tool" => Ok(Self::Tool),
            other => Err(format!(
                "sandbox.scope must be session, turn, or tool, got \"{other}\""
            )),
        }
    }
}

/// What a backend actually does when asked to honour a [`SandboxScope`].
///
/// Three of these are "supported" and mean visibly different things, which is
/// why this is not a boolean: an operator reading `sandbox status` needs to know
/// whether their build cache survives, and "supported" alone does not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxScopeSupport {
    /// The backend holds one sandbox open for this lifetime and re-enters it
    /// per command, so anything a command leaves outside the workspace — build
    /// caches, `/tmp`, installed packages — is there for the next one.
    Held,
    /// The world outlives Archon and is neither created nor destroyed by it, so
    /// every scope names the same durable place and there is no lifetime to
    /// manage. State survives regardless of the scope.
    Durable,
    /// Every command builds and destroys its own world. Honest for `tool`, and
    /// under any longer scope it would be a lie.
    PerCommand,
    /// This backend cannot honour this lifetime. The string says why, in terms
    /// an operator can act on.
    Unsupported(String),
}

impl SandboxScopeSupport {
    /// `Err(reason)` for the one variant a configuration must not load with.
    pub fn into_result(self) -> Result<Self, String> {
        match self {
            Self::Unsupported(reason) => Err(reason),
            supported => Ok(supported),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxCommandRequest {
    pub command: String,
    pub working_dir: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub env: Vec<(String, String)>,
    /// The session that owns this command. Part of a held sandbox's identity so
    /// two sessions sharing one process cannot land in one another's world.
    pub session_id: String,
    /// The agent turn this command belongs to, when the caller has turns.
    ///
    /// `None` is a real answer — the workflow CLI and every direct construction
    /// site have no turn loop — and it is emphatically not an identity. Two
    /// unrelated callers that both answer `None` must never share a sandbox on
    /// the strength of it, so a backend under `turn` scope holds nothing for a
    /// request that cannot name its turn.
    pub turn_id: Option<String>,
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

    /// What this backend does when asked to honour `scope`.
    ///
    /// Required rather than defaulted, for the reason `terminal` is: the
    /// configuration must not assume on a backend's behalf. `sandbox.scope`
    /// spent its whole life validated and read by nobody, so a `docker` session
    /// destroyed its container after every command whatever the operator had
    /// set — which is the failure a default here would reproduce. A backend
    /// that cannot hold a sandbox for a lifetime says so, and the configuration
    /// fails to load rather than silently doing something else.
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport;

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
