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
}

impl LiveAgentDispatch {
    pub(super) fn new(client: LiveV2AgentClient) -> Self {
        Self {
            client,
            call_time_budget_override: None,
        }
    }

    /// `0` keeps the derived bound; anything else is the budget in seconds.
    pub(super) fn with_call_time_budget_secs(mut self, secs: u32) -> Self {
        self.call_time_budget_override = budget_override(secs);
        self
    }
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
    fn repository_audit(&self) -> Option<archon_workflow::repository_audit::runtime::AuditRuntime> { self.client.audit.clone() }

    fn call_time_budget(&self) -> Option<std::time::Duration> {
        self.call_time_budget_override
            .or_else(|| derived_budget(self.client.timeout_secs()))
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
        if execution.call.options.extra.contains_key("repository_audit_contract") {
            return super::workflow_live_v2_script::AuditDispatch(self.client.for_audit()).run_call(
                task, repository_root, execution, adapter, v2_store, task_universe,
            ).await;
        }
        run_single_v2_agent_call_in_repository(
            task,
            repository_root,
            execution,
            adapter,
            &self.client,
            v2_store,
            task_universe,
            // The port resolves the override before it gets here, so there is
            // never a second root to fall back to.
            None,
            false,
        )
        .await
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
