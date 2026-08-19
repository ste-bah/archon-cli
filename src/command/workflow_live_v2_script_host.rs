use super::*;

pub(super) struct WorkflowScriptHost {
    pub(super) scaffold_hash: String,
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
    /// Run one `runTool` host call.
    pub(super) async fn run_script_tool(
        &self,
        payload: &str,
    ) -> archon_workflow::WorkflowResult<String> {
        let host = match self.tool_host.get() {
            Some(host) => Arc::clone(host),
            None => {
                let built = Arc::new(
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
                    )?,
                );
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

#[path = "workflow_live_v2_script_host_events.rs"]
mod workflow_live_v2_script_host_events;
#[path = "workflow_live_v2_script_host_exec.rs"]
mod workflow_live_v2_script_host_exec;
#[path = "workflow_live_v2_script_host_state.rs"]
mod workflow_live_v2_script_host_state;
