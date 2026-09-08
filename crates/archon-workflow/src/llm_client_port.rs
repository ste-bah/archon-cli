//! The port through which a live workflow run reaches an LLM.
//!
//! A live run needs a client that can dispatch subagents. Building one means
//! reading `ArchonConfig`, resolving a provider, installing a subagent executor
//! and wrapping the result in a pipeline adapter — all of which lives in
//! `archon-core` and `archon-pipeline`, neither of which this crate may depend
//! on: `archon-core` depends on `archon-topology`, which depends on this crate.
//!
//! So the direction is inverted. This crate declares what it needs — a client
//! ([`WorkflowLlmClient`]), the shape of one call ([`WorkflowAgentCall`]), and a
//! way to obtain the client ([`WorkflowLlmClientFactory`]) — and the host
//! supplies all three. The host's `archon_pipeline::runner::LlmClient` is one
//! implementation behind an adapter, and nothing here names it.
//!
//! Same shape as `archon-knowledge`'s `CodeSearch`: a narrow trait owned by the
//! layer that consumes it, implemented by the layer that has the dependencies.

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::WorkflowResult;

/// A host-owned provider environment, carried through this port unread.
///
/// The host resolves a run's provider credentials once and needs them attached
/// to every agent call it dispatches. This crate is only the carrier: it never
/// reads the contents and deliberately cannot. A shape it could read is a shape
/// it could log, serialise into a run artefact, or print from a derived
/// `Debug` — and the values inside are secrets. The `Debug` impl below prints a
/// placeholder for that reason.
///
/// The host downcasts back to its own type at the far end. Only the host both
/// constructs and reads these, so a mismatch is unreachable in practice; the
/// adapter that reads it must still fail the call rather than silently drop the
/// environment, because an agent that runs without its credentials fails later
/// and more confusingly than one that never starts.
#[derive(Clone)]
pub struct WorkflowProviderEnv(Arc<dyn Any + Send + Sync>);

impl WorkflowProviderEnv {
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// The host's own type back, or `None` if this handle carries something
    /// else.
    pub fn downcast_ref<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl fmt::Debug for WorkflowProviderEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkflowProviderEnv(<opaque>)")
    }
}

/// How far an agent's tools may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowAgentToolAccess {
    ReadOnly,
    Full,
}

/// Identity and acceptance settings of the agent executing one call.
#[derive(Clone, Debug)]
pub struct WorkflowAgentSpec {
    pub key: String,
    pub display_name: String,
    pub model: String,
    pub phase: u32,
    pub critical: bool,
    pub parallelizable: bool,
    pub quality_threshold: f64,
    pub tool_access: WorkflowAgentToolAccess,
}

/// Everything one agent invocation needs.
///
/// Cloned per attempt by the transient-retry path, so every field is owned.
#[derive(Clone, Debug)]
pub struct WorkflowAgentCall {
    pub session_id: String,
    pub task: String,
    /// Working directory the agent runs in. `None` leaves the host's default.
    pub cwd: Option<PathBuf>,
    pub ordinal: usize,
    pub attempt: usize,
    pub agent: WorkflowAgentSpec,
    pub messages: Vec<Value>,
    pub system: Vec<Value>,
    pub tools: Vec<Value>,
    pub allowed_tools: Vec<String>,
    pub timeout_secs: Option<u64>,
    /// Keep the agent in the foreground past the host's auto-background
    /// threshold. Workflow stages are awaited, so a backgrounded agent is a
    /// lost result rather than a slow one.
    pub disable_auto_background: bool,
    /// Absolute directories this call's agent may write, as the host resolved
    /// them before the agent existed.
    ///
    /// Empty means the host declared nothing, which is not the same as
    /// permitting everything — it is the input to that decision, and the host
    /// adapter decides. Carried as a field rather than left in the prompt
    /// because prose is not enforceable: the artifact contract already reached
    /// the agent as text, and text is what an agent that wrote five files into
    /// the wrong checkout had been given.
    ///
    /// Never widened by anything the agent says. See
    /// `crate::v2::project_artifact_write_roots` for why the set is the project
    /// and repository roots rather than the directory the agent runs in.
    pub write_roots: Vec<String>,
    pub provider_env: Option<WorkflowProviderEnv>,
}

/// One tool invocation an agent performed, as reported back.
#[derive(Clone, Debug)]
pub struct WorkflowAgentToolUse {
    pub tool_name: String,
    pub input: Value,
    pub output: Value,
}

/// What one agent invocation produced.
#[derive(Clone, Debug, Default)]
pub struct WorkflowAgentOutcome {
    pub content: String,
    pub tool_uses: Vec<WorkflowAgentToolUse>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub stop_reason: Option<String>,
}

/// The LLM a live workflow run talks to.
///
/// Futures are `Send`: stage execution runs inside spawned tasks, so anything
/// narrower would not be usable there. This is the opposite constraint from
/// [`WorkflowLlmClientFactory`] below, and for the opposite reason — building a
/// client happens once, before any task is spawned.
///
/// Errors cross as [`WorkflowError::Port`](crate::error::WorkflowError::Port),
/// which is `#[error(transparent)]`, so the host's message reaches the user and
/// the retry classifier unchanged. That matters: transient-provider detection
/// reads the message text.
#[async_trait]
pub trait WorkflowLlmClient: Send + Sync {
    /// Resolved operator policy carried by the composition layer.
    fn repository_audit_policy(&self) -> Option<crate::repository_audit::budget::AuditPolicy> { None }

    /// The provider actually serving this client, when it knows.
    fn provider_id(&self) -> Option<String> {
        None
    }

    /// Resolves a tier alias (`"sonnet"`) to whatever the provider calls it.
    fn resolve_model_alias(&self, model: &str) -> String {
        model.to_string()
    }

    /// A plain completion, with no tool loop. The planner path uses this.
    async fn send_message(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome>;

    /// Explicit sampling must be honored or refused, never silently defaulted.
    async fn send_message_with_temperature(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        Err(crate::error::WorkflowError::port(std::io::Error::other(
            "client does not support explicit sampling",
        )))
    }

    /// A full agent invocation, which the host may serve with a real
    /// tool-capable subagent.
    ///
    /// The default degrades to a plain completion so a client that only knows
    /// how to complete is still usable everywhere; hosts with a subagent
    /// runtime override it.
    async fn run_agent(&self, call: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        let model = call.agent.model.clone();
        self.send_message(call.messages, call.system, call.tools, &model)
            .await
    }
}

/// What the host needs to know to build a client for one run.
///
/// Configuration is not here — it belongs to the implementation, which the host
/// constructs already bound to the config and environment it read.
///
/// Owned, not borrowed. A borrowed form gives the struct a lifetime parameter,
/// and `async_trait` cannot then prove the boxed future `Send` ("implementation
/// of `Send` is not general enough"). Owning it also leaves the port usable from
/// a spawned task, which live runs already are. The cost is one allocation per
/// run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLlmClientRequest {
    /// Working directory the run executes against.
    pub cwd: PathBuf,
    /// Provider-usage origin tag, for cost attribution.
    pub origin: String,
    /// Session identity the run's subagents share.
    pub session_id: String,
}

/// Builds the LLM client for a live workflow run.
///
/// This used to carry a `C: ?Sized` client parameter because the client type
/// could not be named here. [`WorkflowLlmClient`] now names it, so the
/// parameter is gone: it had exactly one instantiation and only obscured what
/// the factory returns.
///
/// `?Send` on the returned future, not on the implementor. The bin crate's
/// implementation starts MCP servers, and that path holds a `tokio` `Notified`
/// across an await, which rustc cannot prove `Send` for
/// (rust-lang/rust#100013). Requiring `Send` here would not make that future
/// safe to spawn — it would only fail to compile. The constraint this imposes
/// is real and narrow: a caller must build its client before entering a
/// spawned task, which is what the live CLI path already does. The implementor
/// itself is still `Send + Sync`, so the port can be held across one.
#[async_trait(?Send)]
pub trait WorkflowLlmClientFactory: Send + Sync {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> WorkflowResult<Arc<dyn WorkflowLlmClient>>;
}
