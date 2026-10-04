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
        call_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<Option<WorkflowV2Result>> {
        if execution.call.method == WorkflowV2HostMethod::HostCommand {
            return self
                .execute_host_command(execution, generation)
                .await
                .map(Some);
        }
        // Boxed (#246): the race below would otherwise hold a second copy.
        let work = Box::pin(execute_v2_live_call(
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
        ));
        if !self.call_fenced(execution) {
            return work.await.map(Some);
        }
        // A fixed run is also stopped by its generation moving on; any other
        // run by a pause or cancel alone.
        let raced = archon_workflow::control_race::until_run_stops_from_generation(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
            generation,
            work,
        );
        // Round 5 (#253): a lifecycle edit to the running run abandons the
        // call; `None` tells the caller to dispatch it again.
        tokio::select! {
            biased;
            result = raced => result.map(Some),
            () = self.superseded_watch(execution, call_generation) => Ok(None),
        }
    }

    /// Whether lifecycle edits fence this call. A host command and a write
    /// call land as a whole and are never dropped part way.
    fn call_fenced(&self, execution: &WorkflowV2CallExecution) -> bool {
        let fixed_agent = self.fixed_decomposition_state_present()
            && execution.call.method == WorkflowV2HostMethod::Agent;
        execution.call.method != WorkflowV2HostMethod::HostCommand
            && (fixed_agent || execution.call.write_mode.is_none())
    }

    /// The run generation a call is dispatched under, read just before it.
    pub(super) fn call_generation(&self) -> archon_workflow::WorkflowResult<u64> {
        Ok(self
            .runner
            .workflow_store
            .load_state(&self.runner.run_id)?
            .generation)
    }

    /// Issue-253 round 5: a restart, item restart or force-accept on the
    /// running run moved its generation while this call was in flight, and
    /// this executor still owns the run. The call's result predates the edit
    /// and is never published. A pause, cancel or new executor is run
    /// control, not supersession, and is answered by the control race.
    pub(super) fn call_superseded(
        &self,
        execution: &WorkflowV2CallExecution,
        call_generation: Option<u64>,
    ) -> bool {
        let Some(dispatched) = call_generation else {
            return false;
        };
        self.call_fenced(execution)
            && self
                .runner
                .workflow_store
                .load_state(&self.runner.run_id)
                .is_ok_and(|run| {
                    run.status == archon_workflow::RunStatus::Running
                        && run.generation != dispatched
                        && run.execution_owned_at(dispatched)
                })
    }

    async fn superseded_watch(
        &self,
        execution: &WorkflowV2CallExecution,
        call_generation: Option<u64>,
    ) {
        let mut watch = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            watch.tick().await;
            if self.call_superseded(execution, call_generation) {
                return;
            }
        }
    }

    /// Drop a superseded call's in-flight state and run the call again from
    /// the top, under the run's new generation. Boxed: this recurses.
    pub(super) fn redispatch_superseded<'a>(
        &'a self,
        execution: &'a WorkflowV2CallExecution,
        method: String,
        payload: String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = archon_workflow::WorkflowResult<String>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.clear_inflight(&execution.call.id);
            drop(self.take_call_sessions(&execution.call.id));
            self.emit_v2_event(
                WorkflowEventKind::StageStarted,
                serde_json::json!({
                    "event": "call_superseded",
                    "call_id": execution.call.id.clone(),
                    "method": execution.call.method.as_str(),
                }),
            );
            self.execute(method, payload).await
        })
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_pause_tests.rs"]
mod pause_tests;

#[cfg(test)]
#[path = "workflow_live_v2_script_host_resume_tests.rs"]
mod resume_tests;
