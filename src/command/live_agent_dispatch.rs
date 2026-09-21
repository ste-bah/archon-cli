//! Host side of `archon_workflow::agent_dispatch_port`.
//!
//! `archon-workflow` cannot name `LiveV2AgentClient` (see the port's module
//! doc), so this file is where the write layer's dispatch port meets the live
//! agent client. It is one impl block forwarding to
//! `run_single_v2_agent_call_in_repository`, which already speaks
//! `WorkflowResult` — nothing is translated.
//!
//! Deliberately not named `workflow_*`. Every `src/command/workflow*.rs` file
//! is destined for `crates/archon-workflow`, and none of them may name
//! `archon_tools::provider_env` or the live runner; keeping the adapter outside
//! that prefix makes the invariant a one-line grep rather than a convention.
//! Same reason `pipeline_workflow_llm.rs`, `tui_workflow_ui_sink.rs` and
//! `lifecycle_script_host.rs` sit outside it.

use archon_workflow::agent_dispatch_port::WorkflowAgentDispatch;
use archon_workflow::error::WorkflowResult;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::{
    WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2Result, WorkflowV2ResultStore,
};
use async_trait::async_trait;

use super::workflow_live_v2_client::LiveV2AgentClient;
use super::workflow_live_v2_host_dispatch::run_single_v2_agent_call_in_repository;

/// Presents the live agent client through the workflow dispatch port.
///
/// Owns a clone rather than borrowing: `LiveV2AgentClient` is `Clone` over an
/// `Arc` and a handful of strings, one clone is taken per fan-out call rather
/// than per branch, and owning it keeps the trait object free of a lifetime
/// parameter at every call site that threads it through the write layer.
pub(super) struct LiveAgentDispatch {
    client: LiveV2AgentClient,
    /// Operator-set total budget for one write call; `None` derives it.
    call_time_budget_override: Option<std::time::Duration>,
    /// `workflow.generated.submit_grace_calls`, handed to each write session's
    /// read guard with the tests its task declares.
    submit_grace_calls: u32,
    /// `workflow.generated.timeout_retry_budget_secs`.
    timeout_retry_budget: Option<std::time::Duration>,
    /// `workflow.generated.resume_memory_calls`.
    resume_memory_calls: usize,
    /// `workflow.generated.verification_branch_timeout_secs`: the bound a
    /// verifier's run of a declared test lives under, and so the bound the
    /// host's own baseline run of the same command gets (Obs-31).
    verification_branch_timeout: Option<std::time::Duration>,
}

impl LiveAgentDispatch {
    pub(super) fn new(client: LiveV2AgentClient) -> Self {
        let defaults = archon_core::config::GeneratedWorkflowConfig::default();
        Self {
            client,
            call_time_budget_override: None,
            submit_grace_calls: defaults.submit_grace_calls,
            timeout_retry_budget: budget_override(defaults.timeout_retry_budget_secs),
            resume_memory_calls: defaults.resume_memory_calls as usize,
            verification_branch_timeout: budget_override(defaults.verification_branch_timeout_secs),
        }
    }

    /// `0` keeps the derived bound; anything else is the budget in seconds.
    pub(super) fn with_call_time_budget_secs(mut self, secs: u32) -> Self {
        self.call_time_budget_override = budget_override(secs);
        self
    }

    pub(super) fn with_generated_config(
        mut self,
        config: &archon_core::config::GeneratedWorkflowConfig,
    ) -> Self {
        self.submit_grace_calls = config.submit_grace_calls;
        self.timeout_retry_budget = budget_override(config.timeout_retry_budget_secs);
        self.resume_memory_calls = config.resume_memory_calls as usize;
        self.verification_branch_timeout = budget_override(config.verification_branch_timeout_secs);
        self.with_call_time_budget_secs(config.write_call_time_budget_secs)
    }
}

/// The per-dispatch timeout the write layer asked for on this one call, if any.
fn dispatch_timeout_override(execution: &WorkflowV2CallExecution) -> Option<u64> {
    execution
        .call
        .options
        .extra
        .get(archon_workflow::agent_dispatch_port::DISPATCH_TIMEOUT_OVERRIDE_KEY)
        .and_then(serde_json::Value::as_u64)
        .filter(|secs| *secs > 0)
}

/// How much total wall clock a call gets, as a multiple of one dispatch's.
///
/// Expressed against the operator's own timeout rather than as a duration, so
/// the bound moves with the setting instead of contradicting it. Three means a
/// call may spend its attempt and two corrections' worth of time; a call still
/// going after that is not converging, and the retry budgets above it — up to
/// thirteen size re-asks, plus transport retries that deliberately do not
/// consume that budget — would otherwise let it run for a day under a
/// two-hour timeout.
const CALL_TIME_BUDGET_DISPATCHES: u64 = 3;

fn budget_override(secs: u32) -> Option<std::time::Duration> {
    (secs > 0).then(|| std::time::Duration::from_secs(u64::from(secs)))
}

fn derived_budget(timeout_secs: Option<u64>) -> Option<std::time::Duration> {
    timeout_secs.map(|secs| {
        std::time::Duration::from_secs(secs.saturating_mul(CALL_TIME_BUDGET_DISPATCHES))
    })
}

#[async_trait]
impl WorkflowAgentDispatch for LiveAgentDispatch {
    fn repository_audit(&self) -> Option<archon_workflow::repository_audit::runtime::AuditRuntime> {
        self.client.audit.clone()
    }

    fn call_time_budget(&self) -> Option<std::time::Duration> {
        self.call_time_budget_override
            .or_else(|| derived_budget(self.client.timeout_secs()))
    }

    /// The per-dispatch timeout the client puts on every agent call — the
    /// configured `host_call_timeout_secs` on the generated path — which is
    /// what actually ends a session.
    fn dispatch_timeout(&self) -> Option<std::time::Duration> {
        self.client
            .timeout_secs()
            .map(std::time::Duration::from_secs)
    }

    fn timeout_retry_budget(&self) -> Option<std::time::Duration> {
        self.timeout_retry_budget
    }

    fn resume_memory_calls(&self) -> usize {
        self.resume_memory_calls
    }

    /// The smaller of the verifier's branch timeout and the per-dispatch
    /// timeout: a baseline run of a declared command may take no longer than
    /// the verifier that will run it again would be given.
    fn baseline_test_timeout(&self) -> Option<std::time::Duration> {
        match (self.verification_branch_timeout, self.dispatch_timeout()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// A host-run command in a branch worktree builds where the coder's own
    /// build and test calls do: a slot leased from the shared build-cache
    /// pool (`bash_build_cache`), with the toolchain variables the repository
    /// markers under `working_root` call for. The lease is held in `hold`
    /// until the caller drops the environment. No pool (an interactive
    /// session, or none installed yet) means no overrides.
    async fn host_command_env(
        &self,
        working_root: &std::path::Path,
    ) -> archon_workflow::agent_dispatch_port::HostCommandEnv {
        let Some(pool) = archon_tools::build_cache_lease::shared_build_cache_pool() else {
            return Default::default();
        };
        let lease = match pool.acquire().await {
            Ok(lease) => lease,
            Err(error) => {
                tracing::warn!(%error, "baseline tests: build cache lease unavailable");
                return Default::default();
            }
        };
        let vars =
            archon_tools::build_cache_env::cache_env_for_repository(working_root, lease.dir(), &[]);
        archon_workflow::agent_dispatch_port::HostCommandEnv {
            vars,
            hold: Some(Box::new(lease)),
        }
    }

    async fn run_call(
        &self,
        task: &str,
        repository_root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        v2_store: Option<&WorkflowV2ResultStore>,
        task_universe: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        if execution
            .call
            .options
            .extra
            .contains_key("repository_audit_contract")
        {
            return super::workflow_live_v2_script::AuditDispatch(self.client.for_audit())
                .run_call(
                    task,
                    repository_root,
                    execution,
                    adapter,
                    v2_store,
                    task_universe,
                )
                .await;
        }
        // A retry of a timed-out branch runs under the budget the write layer
        // set for it, attributed to that setting in `transport.jsonl`.
        let client = match dispatch_timeout_override(execution) {
            Some(secs) => self
                .client
                .with_timeout_secs(Some(secs), "timeout_retry_budget_secs"),
            None => self.client.clone(),
        };
        let repository_root_for_guard = repository_root.clone();
        // Pinned to the heap before the guard scopes below take it by value.
        // This future is the whole live agent call, hundreds of kilobytes in a
        // debug build, and each `scope_*` wrapper moves its argument into a
        // new state machine — so left inline, this one poll frame carried
        // four copies of it (measured at 1.3-1.5 MiB under lldb) and overflowed
        // libtest's 2 MiB test thread, and tokio's 2 MiB blocking thread that
        // hosts the script runtime, in eight of the bin's workflow tests. Boxed,
        // the wrappers move a pointer.
        let call = Box::pin(run_single_v2_agent_call_in_repository(
            task,
            repository_root,
            execution,
            adapter,
            &client,
            v2_store,
            task_universe,
            // The port resolves the override before it gets here, so there is
            // never a second root to fall back to.
            None,
            false,
        ));
        // The tests the task declares, so the session's read guard can tell
        // the agent to submit once they have all passed. Inert for an item
        // that declares none, and for a session with no guard (read-only).
        let call = archon_tools::workflow_read_guard::scope_focused_tests(
            archon_tools::workflow_read_guard::FocusedTestPlan::new(
                archon_workflow::agent_dispatch_port::declared_focused_tests(&execution.input),
                self.submit_grace_calls,
            ),
            call,
        );
        // The paths the task forbids (Issue-30), stamped by the write layer,
        // so the same guard refuses a Write/Edit at one of them before the
        // file changes. Same shape as the focused tests, for the same
        // reason: the pipeline builds the guard per session and cannot be
        // handed the list directly. Inert for an item whose tasks forbid
        // nothing; `repository_root` is the branch worktree the call runs in.
        let call = archon_tools::workflow_read_guard::scope_forbidden_paths(
            archon_tools::workflow_read_guard::ForbiddenPathScope::new(
                &archon_workflow::agent_dispatch_port::declared_forbidden_paths(&execution.input),
                &archon_workflow::agent_dispatch_port::forbidden_path_roots(
                    &execution.input,
                    repository_root_for_guard.as_deref(),
                ),
            ),
            call,
        );
        // The declared target set (Issue-64), widened by the baseline and
        // stamped by the write layer, so the same guard refuses a Write/Edit
        // at a worktree path outside it before the file changes rather than
        // the gate dropping the change afterwards. Inert for a call with no
        // stamp; `repository_root` is the branch worktree the call runs in.
        let call = archon_tools::workflow_read_guard::scope_declared_targets(
            archon_tools::workflow_read_guard::DeclaredTargetScope::new(
                &archon_workflow::agent_dispatch_port::declared_targets(&execution.input),
                repository_root_for_guard.as_deref(),
            ),
            call,
        );
        if let Some(store) = v2_store {
            archon_tools::workflow_read_guard::scope_read_set(
                archon_workflow::v2::write_read_set::path(store, &execution.call.id),
                call,
            )
            .await
        } else {
            call.await
        }
    }

    fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        self.client.fanout_parallelism(requested)
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn operator_budget_overrides_the_derived_bound_and_zero_keeps_it() {
        assert_eq!(
            derived_budget(Some(100)),
            Some(std::time::Duration::from_secs(300))
        );
        assert_eq!(derived_budget(None), None);
        assert_eq!(budget_override(0), None);
        assert_eq!(
            budget_override(900).or_else(|| derived_budget(Some(100))),
            Some(std::time::Duration::from_secs(900))
        );
    }
}
