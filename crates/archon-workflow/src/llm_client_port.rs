//! The port through which a live workflow run obtains its LLM client.
//!
//! A live run needs a client that can dispatch subagents. Building one means
//! reading `ArchonConfig`, resolving a provider, installing a subagent executor
//! and wrapping the result in a pipeline adapter — all of which lives in
//! `archon-core` and `archon-pipeline`, neither of which this crate may depend
//! on: `archon-core` depends on `archon-topology`, which depends on this crate.
//!
//! So the direction is inverted. This crate declares what it needs and the host
//! supplies it. The client type stays a parameter because naming it would mean
//! naming `archon_pipeline::runner::LlmClient`; the workflow layer only needs to
//! know that something can be constructed and handed on.
//!
//! Same shape as `archon-knowledge`'s `CodeSearch`: a narrow trait owned by the
//! layer that consumes it, implemented by the layer that has the dependencies.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::WorkflowResult;

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
/// `C` is the host's client type, left open because this crate cannot name it.
/// It is `?Sized` so a host can implement the port for a trait object.
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
pub trait WorkflowLlmClientFactory<C: ?Sized>: Send + Sync {
    async fn build_client(&self, request: WorkflowLlmClientRequest) -> WorkflowResult<Arc<C>>;
}
