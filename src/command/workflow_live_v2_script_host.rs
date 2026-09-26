use super::*;

pub(super) struct WorkflowScriptHost {
    pub(super) scaffold_hash: String,
    /// Which envelope a host-call result is rendered into for the script:
    /// the deduplicated one for v3 `export const meta` scripts, the compat one
    /// (every nested copy kept) for the decomposed dialect, whose Rust driver
    /// reads the same string.
    pub(super) envelope_shape: ScriptEnvelopeShape,
    pub(super) runner: WorkflowV2ScriptRunner,
    pub(super) accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
    /// Registry and permission gate for `runTool` (#189 Phase 4).
    ///
    /// Lazy: building it walks the working tree, and most runs never call a
    /// tool. Once built it is shared for the run, because doing that per call
    /// would make `tool()` slower than the model round-trip it replaces.
    pub(super) tool_host: std::sync::OnceLock<
        Arc<crate::command::workflow_live::workflow_script_tools::ScriptToolHost>,
    >,
    pub(super) tool_budget:
        Arc<std::sync::Mutex<crate::command::workflow_live::workflow_script_tools::ToolCallBudget>>,
}

impl WorkflowScriptHost {
    /// A stored result in the envelope shape this run's script reads. A
    /// refused remediation verdict carries the host's cross-owner plan
    /// (Issue-107), computed here on every answering path -- run, replayed,
    /// drifted or history -- and never persisted.
    pub(super) fn result_view(
        &self,
        call: &WorkflowV2HostCall,
        result: &WorkflowV2Result,
    ) -> archon_workflow::WorkflowResult<String> {
        let root = self
            .runner
            .runtime
            .target_repository_root
            .as_deref()
            .map(std::path::Path::new);
        let planned = archon_workflow::v2::script::remediation_escalation::with_escalation_plan(
            call,
            result,
            self.runner.task_universe.as_ref(),
            root,
        );
        result_view_json_shaped(planned.as_ref().unwrap_or(result), self.envelope_shape)
    }

    /// Run one `runTool` host call.
    pub(super) async fn run_script_tool(
        &self,
        payload: &str,
    ) -> archon_workflow::WorkflowResult<String> {
        let host = match self.tool_host.get() {
            Some(host) => Arc::clone(host),
            None => {
                let mut built =
                    crate::command::workflow_live::workflow_script_tools::ScriptToolHost::new(
                        self.runner
                            .runtime
                            .target_repository_root
                            .as_deref()
                            .map_or_else(
                                || std::env::current_dir().unwrap_or_default(),
                                std::path::PathBuf::from,
                            ),
                        self.runner.run_id.clone(),
                    )?;
                if self.runner.client.audit.is_some() {
                    built.require_audited_writes();
                }
                let built = Arc::new(built);
                // A concurrent caller may have won; either instance is
                // equivalent, so the loser's is simply dropped.
                let _ = self.tool_host.set(Arc::clone(&built));
                self.tool_host.get().map_or(built, Arc::clone)
            }
        };
        crate::command::workflow_live::workflow_script_tools::execute_run_tool(
            &host,
            &self.tool_budget,
            payload,
        )
        .await
    }
}

#[path = "workflow_live_v2_script_host_command.rs"]
mod workflow_live_v2_script_host_command;
#[path = "workflow_live_v2_script_host_events.rs"]
mod workflow_live_v2_script_host_events;
#[path = "workflow_live_v2_script_host_exec.rs"]
mod workflow_live_v2_script_host_exec;
#[path = "workflow_live_v2_script_host_interrupt.rs"]
mod workflow_live_v2_script_host_interrupt;
use workflow_live_v2_script_host_interrupt::control_interruption_reason;
#[path = "workflow_live_v2_script_host_history.rs"]
mod workflow_live_v2_script_host_history;
#[path = "workflow_live_v2_script_host_state.rs"]
mod workflow_live_v2_script_host_state;

#[path = "workflow_live_v2_script_host_audit.rs"]
mod workflow_live_v2_script_host_audit;
