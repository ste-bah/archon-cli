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
}

impl LiveAgentDispatch {
    pub(super) fn new(client: LiveV2AgentClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl WorkflowAgentDispatch for LiveAgentDispatch {
    async fn run_call(
        &self,
        task: &str,
        repository_root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        v2_store: Option<&WorkflowV2ResultStore>,
        task_universe: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
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
        )
        .await
    }

    fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        self.client.fanout_parallelism(requested)
    }
}
