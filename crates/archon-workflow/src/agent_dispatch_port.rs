//! The port through which write-capable fan-out dispatches one agent call.
//!
//! The write layer plans worktrees, enforces ownership, validates changed files
//! against the write plan, merges branches and attributes completion evidence.
//! Exactly one thing in it is not that: the moment a branch hands its execution
//! to an agent and waits for a result. That moment named `LiveV2AgentClient`,
//! which owns an `Arc<dyn WorkflowLlmClient>`, a provider-environment
//! resolution from `archon-tools`, and reaches the live runner for agent
//! identity and tool binding — none of which this crate may depend on.
//!
//! So the direction is inverted, the same way [`crate::llm_client_port`]
//! inverts the LLM and [`crate::lifecycle_host_port`] inverts the script host.
//! The write layer declares the one call it makes; the host supplies it.
//!
//! This is deliberately *not* the LLM port one level up. The host's
//! implementation does substantially more than send a request: it resolves the
//! stored source for the execution, stamps project-artifact requirements,
//! prepares and stamps the provider environment, picks a provider tier from the
//! request, logs rejected output against the result store, and translates one
//! specific agent error into a repairable reduce result rather than a failure.
//! All of that is host policy about how an agent call is made, and none of it
//! belongs to a layer whose job is deciding *which* calls to make.
//!
//! Every parameter is a type this crate already owns, so the port removes
//! exactly one name from the write layer's vocabulary and adds none.
//!
//! Errors cross as whatever the host raises. The host implementation this
//! replaced already returned [`WorkflowResult`], and the write layer routes on
//! the error text — a recoverable branch timeout is detected by string match —
//! so a translation layer here would break that. A host wrapping a foreign
//! error uses [`WorkflowError::port`](crate::error::WorkflowError::port), which
//! is `#[error(transparent)]`.

use async_trait::async_trait;

use crate::error::WorkflowResult;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::agent_adapter::WorkflowV2AgentAdapter;
use crate::v2::call_execution::WorkflowV2CallExecution;
use crate::v2::result::WorkflowV2Result;
use crate::v2::result_store::WorkflowV2ResultStore;

/// Dispatches one workflow agent call and returns its typed result.
#[async_trait]
pub trait WorkflowAgentDispatch: Send + Sync {
    /// Run `execution` as a single agent call.
    ///
    /// `repository_root` is the working directory the agent runs against.
    /// Worktree fan-out passes the branch's sealed workspace here; every other
    /// path passes the run's target repository root, or `None` for the host's
    /// default. The two used to be separate parameters with the override taking
    /// precedence — one value, resolved by the caller that knows which it has.
    ///
    /// `v2_store` is `Some` whenever the call has a result store to resolve
    /// stored source from and log rejected output against. `None` means neither
    /// happens, which is what the read-only artifact path wants.
    async fn run_call(
        &self,
        task: &str,
        repository_root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        v2_store: Option<&WorkflowV2ResultStore>,
        task_universe: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result>;

    /// How many branches this host will run at once, given the cap the call
    /// itself requested.
    ///
    /// The write layer decides how to *shape* a wave; how wide the host will
    /// actually let one get is the host's answer, because it depends on the
    /// configured subagent concurrency. A `requested` value can only narrow the
    /// result, never widen it past what the host allows.
    fn fanout_parallelism(&self, requested: Option<usize>) -> usize;
}
