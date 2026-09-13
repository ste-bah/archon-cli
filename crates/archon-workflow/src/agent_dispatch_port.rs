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
//! is `#[error(transparent)]`. One error is typed rather than routed on text:
//! a call the host's own per-dispatch timer ended must come back as
//! [`WorkflowError::HostCallTimeout`](crate::error::WorkflowError::HostCallTimeout),
//! or the write layer's transport re-ask reads the pipeline's wording as a
//! provider drop and restarts the session under the budget that just cut it.

use async_trait::async_trait;

use crate::error::WorkflowResult;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::agent_adapter::WorkflowV2AgentAdapter;
use crate::v2::call_execution::WorkflowV2CallExecution;
use crate::v2::result::WorkflowV2Result;
use crate::v2::result_store::WorkflowV2ResultStore;

/// `call.options.extra` key carrying a per-dispatch wall-clock override, in
/// seconds. Set by the write layer on the single in-run retry of a timed-out
/// branch; the host applies it in place of its configured per-dispatch timeout
/// for that one call. Named here so both sides spell it the same way.
pub const DISPATCH_TIMEOUT_OVERRIDE_KEY: &str = "host_dispatch_timeout_secs";

/// The focused test commands a write branch's item declares, verbatim, or
/// empty when it declares none. Read from the branch input the same way the
/// source-graph fields are (`focused_verification` first, then the aliases the
/// authored script may have used).
pub fn declared_focused_tests(input: &serde_json::Value) -> Vec<String> {
    let item = input.get("item").unwrap_or(input);
    ["focused_verification", "focused_tests", "focusedTests"]
        .iter()
        .find_map(|key| item.get(*key).and_then(serde_json::Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
        .collect()
}

/// Dispatches one workflow agent call and returns its typed result.
#[async_trait]
pub trait WorkflowAgentDispatch: Send + Sync {
    /// Host-owned audit context, never extracted from an authored envelope.
    fn repository_audit(&self) -> Option<crate::repository_audit::runtime::AuditRuntime> { None }

    /// Wall clock a single call may spend IN TOTAL, across every re-dispatch.
    ///
    /// The timeout the host already applies bounds one dispatch, and a call is
    /// re-dispatched by several independent budgets — a size-rejection re-ask,
    /// transport retries that deliberately do not consume it, a schema repair,
    /// and the port's own transient retry. None of them looks at the clock, and
    /// nothing added their elapsed time together, so a two-hour timeout
    /// permitted a day of work: observed as a single task running 6h20m and
    /// still going when it was killed by hand.
    ///
    /// `None` means unbounded, which is the honest answer for a caller that has
    /// no configured timeout to derive one from. The bound belongs to the
    /// dispatcher rather than the loop because only the dispatcher knows what
    /// the operator asked for; a constant here would be a number nobody chose.
    fn call_time_budget(&self) -> Option<std::time::Duration> {
        None
    }

    /// Wall clock the host allows ONE dispatch before ending it itself.
    ///
    /// This is the limit an agent actually runs into: the host cancels the
    /// session when it passes, whatever the total call budget above still has
    /// left. A write branch was told "240 minutes" from `call_time_budget`
    /// while the host cut its session at 7200 s; the prompt renders the
    /// smaller of the two now. `None` means the host applies no per-dispatch
    /// timeout.
    fn dispatch_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// Wall clock for the one in-run retry of a write branch that timed out
    /// with partial work captured (`workflow.generated.timeout_retry_budget_secs`).
    ///
    /// The retry starts from the captured patch and is told its declared
    /// tests are believed to pass, so it needs minutes, not the hours the
    /// first session had. `None` means no bound beyond [`Self::dispatch_timeout`].
    fn timeout_retry_budget(&self) -> Option<std::time::Duration> {
        None
    }

    /// How many of the previous session's most recent tool calls a resumed,
    /// retried or restarted write session is shown, beside every call the
    /// host refused it (`workflow.generated.resume_memory_calls`).
    fn resume_memory_calls(&self) -> usize {
        crate::v2::write::session_memory::DEFAULT_LAST_CALLS
    }

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
