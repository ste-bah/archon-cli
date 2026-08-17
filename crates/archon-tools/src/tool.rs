use std::path::PathBuf;
use std::sync::Arc;

use archon_observability::AgentActivitySink;
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

/// Whether a tool can mutate files below the session working tree.
///
/// New tools conservatively default to [`Arbitrary`], so a newly registered
/// mutator cannot silently evade working-tree observation. `ExternalOnly`
/// covers network, process, or storage effects that do not write beneath
/// `ToolContext::working_dir`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkingTreeEffect {
    None,
    /// Mutates a caller-declared path but may also create parent directories.
    DeclaredPaths,
    /// Can create, modify, or delete arbitrary paths under the working tree.
    #[default]
    Arbitrary,
    /// Has side effects outside the observed working tree.
    ExternalOnly,
}

impl WorkingTreeEffect {
    pub fn requires_filesystem_observation(self) -> bool {
        matches!(self, Self::DeclaredPaths | Self::Arbitrary)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolRunAdmissionRequest {
    pub session_id: String,
    pub parent_action_id: String,
    pub tool_use_id: String,
    pub attempt: u32,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub permission_level: PermissionLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRunAdmission {
    Allowed,
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRunAttemptOutcome {
    pub session_id: String,
    pub parent_action_id: String,
    pub tool_use_id: String,
    pub attempt: u32,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub permission_level: PermissionLevel,
    pub blocked: bool,
    pub is_error: bool,
    /// Whether `ToolRunAdmissionCallback` ran for this attempt.
    ///
    /// This callback used to fire only when admission ran — i.e. only for
    /// non-`Safe` tools with an admission callback installed. Ambient topology
    /// tracing needs *every* attempt, so the filter was removed and this flag
    /// took its place.
    ///
    /// **A consumer that correlates against admission state must check this
    /// field.** The world-model guardrail does: it looks up the persisted
    /// admission decision by action id, and for an attempt that was never
    /// admitted there is nothing to find. Before this flag existed the absence
    /// of a decision was inferred from the callback simply not firing.
    pub admission_evaluated: bool,
}

pub type ToolRunAdmissionCallback =
    Arc<dyn Fn(ToolRunAdmissionRequest) -> ToolRunAdmission + Send + Sync>;
pub type ToolRunOutcomeCallback = Arc<dyn Fn(ToolRunAttemptOutcome) + Send + Sync>;

// ---------------------------------------------------------------------------
// Agent mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Normal mode -- all tools available.
    #[default]
    Normal,
    /// Plan Mode blocks working-tree mutations by default while allowing its
    /// canonical Plan-safe controls, including TaskCreate, TaskUpdate, and Agent.
    Plan,
}

// ---------------------------------------------------------------------------
// Tool context -- passed to every tool execution
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub session_id: String,
    /// Identity of the subagent this invocation is running inside.
    ///
    /// `session_id` is copied verbatim from parent to child, so it answers
    /// "which session owns this tree" and can never separate one subagent
    /// from its siblings. A tool that must attribute its action to the
    /// caller reads this instead. `None` means the top-level agent made the
    /// call — that is a real answer, not absent data.
    pub subagent_id: Option<String>,
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
    pub sandbox: Option<Arc<dyn archon_permissions::SandboxBackend>>,
    /// Canonical activity stream for TUI/log/persistence consumers. Tools do
    /// not need to know about rendering; dispatch emits lifecycle events here.
    pub activity_sink: Option<Arc<dyn AgentActivitySink>>,
    /// Parent guarded action for per-attempt ToolRun admission.
    pub tool_run_parent_action_id: Option<String>,
    /// Stable provider tool-use identifier for this invocation.
    pub tool_run_tool_use_id: Option<String>,
    /// Zero-based execution attempt; retries increment this value.
    pub tool_run_attempt: u32,
    /// Binary-installed policy callback. Safe tools bypass it.
    pub tool_run_admission: Option<ToolRunAdmissionCallback>,
    /// Records exactly one terminal outcome for each admitted attempt.
    pub tool_run_outcome: Option<ToolRunOutcomeCallback>,
}

impl ToolContext {
    pub fn with_tool_run_attempt(&self, tool_use_id: impl Into<String>, attempt: u32) -> Self {
        let mut context = self.clone();
        context.tool_run_tool_use_id = Some(tool_use_id.into());
        context.tool_run_attempt = attempt;
        context
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("working_dir", &self.working_dir)
            .field("session_id", &self.session_id)
            .field("subagent_id", &self.subagent_id)
            .field("mode", &self.mode)
            .field("extra_dirs", &self.extra_dirs)
            .field("in_fork", &self.in_fork)
            .field("nested", &self.nested)
            .field("cancel_parent", &self.cancel_parent)
            .field("sandbox", &self.sandbox.as_ref().map(|_| "<sandbox>"))
            .field(
                "activity_sink",
                &self.activity_sink.as_ref().map(|_| "<activity_sink>"),
            )
            .field("tool_run_parent_action_id", &self.tool_run_parent_action_id)
            .field("tool_run_tool_use_id", &self.tool_run_tool_use_id)
            .field("tool_run_attempt", &self.tool_run_attempt)
            .field(
                "tool_run_admission",
                &self.tool_run_admission.as_ref().map(|_| "<callback>"),
            )
            .field(
                "tool_run_outcome",
                &self.tool_run_outcome.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tool result
// ---------------------------------------------------------------------------

/// Opaque metadata minted only by the Bash execution implementation.
///
/// Private fields and crate-private construction prevent external tools or
/// model-supplied prose from claiming that a command actually ran.
#[derive(Debug, Clone)]
pub struct AuthoritativeBashExecution {
    session_id: String,
    tool_use_id: String,
    attempt: u32,
    command: String,
    output: String,
    exit_code: i32,
}

impl AuthoritativeBashExecution {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn tool_use_id(&self) -> &str {
        &self.tool_use_id
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    #[serde(skip)]
    authoritative_bash_execution: Option<Box<AuthoritativeBashExecution>>,
}

impl ToolResult {
    pub fn from_parts(content: impl Into<String>, is_error: bool) -> Self {
        Self {
            content: content.into(),
            is_error,
            authoritative_bash_execution: None,
        }
    }

    pub(crate) fn from_authoritative_bash_execution(
        content: String,
        session_id: String,
        tool_use_id: String,
        attempt: u32,
        command: String,
        exit_code: i32,
    ) -> Self {
        Self {
            authoritative_bash_execution: Some(Box::new(AuthoritativeBashExecution {
                session_id,
                tool_use_id,
                attempt,
                command,
                output: content.clone(),
                exit_code,
            })),
            content,
            is_error: exit_code != 0,
        }
    }

    pub fn authoritative_bash_execution(&self) -> Option<&AuthoritativeBashExecution> {
        self.authoritative_bash_execution.as_deref()
    }

    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            authoritative_bash_execution: None,
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
            authoritative_bash_execution: None,
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

    /// Declare whether this tool can mutate files below the session working tree.
    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::default()
    }

    /// Clone this tool restricted to an isolation tier, when it has anything to
    /// restrict (#184 M3).
    ///
    /// `None` by default: most tools do not care how isolated their agent is.
    /// `Bash` does, because building inside a worktree is what costs disk.
    fn with_isolation_tier(&self, _tier: crate::isolation::IsolationTier) -> Option<Box<dyn Tool>> {
        None
    }

    /// Clone this tool with a provider environment overlay when supported.
    fn with_provider_env_source(
        &self,
        _provider_env: crate::provider_env::ProviderEnvSource,
    ) -> Option<Box<dyn Tool>> {
        None
    }
}
