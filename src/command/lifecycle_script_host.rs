//! Host side of `archon_workflow::lifecycle_host_port`.
//!
//! `archon-workflow` cannot name `WorkflowScriptHost` (see the port's module
//! doc), so this file is where the lifecycle driver's host port meets the
//! concrete script host. It is one impl block forwarding three calls; every
//! method already speaks `WorkflowResult`, so nothing is translated here.
//!
//! Deliberately not named `workflow_*`. Every `src/command/workflow*.rs` file
//! is destined for `crates/archon-workflow`, and none of them may name the live
//! runner stack this host drags in; keeping the adapter outside that prefix
//! makes the invariant a one-line grep rather than a convention. Same reason
//! `pipeline_workflow_llm.rs` and `tui_workflow_ui_sink.rs` sit outside it.
//!
//! It is a child of `workflow_live_v2_script` rather than of `workflow_live_v2`
//! because `WorkflowV2ScriptRunner::v2_store` is private to that module — the
//! same scope the driver used to reach it from.

use archon_workflow::WorkflowV2CallRecord;
use archon_workflow::error::WorkflowResult;
use archon_workflow::lifecycle_host_port::LifecycleHost;
use async_trait::async_trait;

use super::WorkflowScriptHost;

#[async_trait]
impl LifecycleHost for WorkflowScriptHost {
    /// Inherent-first method resolution: `WorkflowScriptHost::execute` names
    /// the inherent method, not this one, so this is a forward and not a
    /// recursion.
    async fn execute(&self, method: String, payload: String) -> WorkflowResult<String> {
        WorkflowScriptHost::execute(self, method, payload).await
    }

    fn load_call_record(&self, call_id: &str) -> WorkflowResult<Option<WorkflowV2CallRecord>> {
        self.runner.v2_store.load_call_record(call_id)
    }

    fn pack_reduce_source(&self, source: &serde_json::Value) -> serde_json::Value {
        super::super::workflow_live_v2_data::source_pack_value(source)
    }
}
