// One more inherent `impl WorkflowScriptHost` block, split out of
// `workflow_live_v2_script_host_exec.rs` to hold the 500-line ceiling: how a
// call is dispatched live.

use super::*;

impl WorkflowScriptHost {
    /// Run `execution` live, abandoned the moment the run stops.
    ///
    /// Issue-136: live, a read-only verifier kept making tool calls for
    /// twelve minutes after a pause, because only the fixed decomposition's
    /// agent calls and each write branch were raced against run control. Every
    /// call is raced now except two that must not be dropped part way: a host
    /// command (its own supervisor ends it) and a write call, whose landing --
    /// patch, project data and commit -- stops only between branches; the
    /// write layer races each of its branches, so its agents stop as promptly.
    /// Dropping the call drops the agent session, whose cancellation aborts
    /// the model request and the tool round (`archon-core`'s subagent runner),
    /// and the host ends its Bash trees before recording the interruption.
    pub(super) async fn dispatch_live(
        &self,
        execution: &WorkflowV2CallExecution,
        source_task_graph: Option<&archon_workflow::WorkflowV2SourceTaskGraph>,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
        if execution.call.method == WorkflowV2HostMethod::HostCommand {
            return self.execute_host_command(execution, generation).await;
        }
        let work = execute_v2_live_call(
            &self.runner.task,
            &self.runner.runtime,
            execution.clone(),
            self.runner.adapter.clone(),
            &self.runner.client,
            &self.runner.v2_store,
            &self.runner.workflow_store,
            &self.runner.run_id,
            self.runner.workspace_boundary_supported,
            self.runner.task_universe.as_ref(),
            source_task_graph,
            self.runner.raw_outcomes_allowed,
        );
        let fixed_agent = self.fixed_decomposition_state_present()
            && execution.call.method == WorkflowV2HostMethod::Agent;
        if !fixed_agent && execution.call.write_mode.is_some() {
            return work.await;
        }
        // A fixed run is also stopped by its generation moving on; any other
        // run by a pause or cancel alone.
        archon_workflow::control_race::until_run_stops_from_generation(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
            generation,
            work,
        )
        .await
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_pause_tests.rs"]
mod pause_tests;

#[cfg(test)]
#[path = "workflow_live_v2_script_host_resume_tests.rs"]
mod resume_tests;
