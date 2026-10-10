//! The fixed executor's answer to an operational ending (Issue #255): retry
//! the same call while it makes progress, else pause the run. The contract
//! and the policy live in `workflow_host_command_operational`.
use super::*;
use crate::command::workflow_host_command_operational::{
    NextStep, OperationalAttempt, OperationalReport, next_step, pause_for_stall,
    pause_for_unsettled_publish, pause_run, record_retry, require_run_owned,
};
use crate::command::workflow_host_command_supervisor::HostCommandControlHandle;

impl FixedHostCommandExecutor {
    async fn execute_process_with_run_control(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
        handle: HostCommandControlHandle,
        expected_generation: u64,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt(
                    "fixed HostCommand run root has no UTF-8 run id".to_string(),
                )
            })?
            .to_string();
        // A run paused meanwhile reports the pause, never a cancellation.
        crate::command::workflow_host_command_operational::require_run_owned(
            &store,
            &run_id,
            expected_generation,
        )?;
        let work = self.process.execute(request, control);
        tokio::pin!(work);
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                result = &mut work => return result,
                _ = poll.tick() => {
                    let Ok(run) = store.load_state(&run_id) else {
                        continue;
                    };
                    if let Some(signal) = crate::command::workflow_host_command_operational::supervisor_signal(
                        &store,
                        &run,
                        expected_generation,
                    ) {
                        handle.signal(signal)?;
                        return work.await;
                    }
                }
            }
        }
    }

    /// Runs `command` until it completes. A timeout or an incomplete,
    /// resumable exit runs it again under `next_step`; when that stops, the
    /// run is paused and the call ends with the pause's control error, so it
    /// is recorded as interrupted and re-run on resume. A completed attempt,
    /// whatever its exit code, and every other error return unchanged.
    pub(super) async fn execute_with_operational_retry(
        &self,
        command: &ResolvedHostCommand,
        call_id: &str,
        expected_generation: u64,
        secrets: &HostSecrets,
        anchor: &crate::command::workflow_host_staging_anchor::StagingAnchor,
        pause: &crate::command::workflow_host_staging_pause::StagingPause,
        teardown: &crate::command::workflow_host_command_teardown_latch::TeardownLatch,
    ) -> WorkflowResult<crate::command::workflow_host_secrets::SealedProcessOutput> {
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("fixed HostCommand run root has no UTF-8 run id".into())
            })?
            .to_string();
        let mut history: Vec<OperationalAttempt> = Vec::new();
        // A call may be executed again after resume, when its attempt count
        // starts over. Keep durable spills from separate executions disjoint.
        let execution_id = uuid::Uuid::new_v4();
        loop {
            let attempt = history.len() as u32 + 1;
            if attempt > 1 {
                require_run_owned(&store, &run_id, expected_generation)?;
                // The killed attempt's partial staging must not reach the
                // audit, which requires the staged set to match the manifest.
                // Cleared through the anchor: a link the killed attempt left
                // is removed, never followed, and the run pauses when it cannot be.
                anchor.reset().map_err(|error| {
                    pause.pause(
                        anchor.root(),
                        "staging could not be cleared for the retry",
                        &secrets.text(&error.to_string()),
                        true,
                    )
                })?;
            }
            // Every attempt's tree reports to the call's latch, so sealing
            // after a cancellation waits for it (#297 round 8).
            let (control, handle) = HostCommandControl::tracked(teardown.clone());
            let started = std::time::Instant::now();
            let mut attempt_command = command.clone();
            if let Some(directory) = &command.spill_dir {
                attempt_command.spill_dir =
                    Some(directory.join(format!("attempt-{attempt}-{execution_id}")));
            }
            let observed = self
                .execute_process_with_run_control(
                    attempt_command,
                    control,
                    handle,
                    expected_generation,
                )
                .await;
            let observed = match observed {
                Ok(observed) => observed,
                Err(error @ (WorkflowError::Io { .. } | WorkflowError::HostOperational(_))) => {
                    let evidence = secrets.text(&error.to_string());
                    let resume = format!("archon workflow resume --live --yes {run_id}");
                    let message = format!(
                        "host command '{}' stalled: {evidence}; repair the host I/O and resume: {resume}",
                        command.command_id
                    );
                    let event = archon_workflow::control_pause::pause_with_evidence(
                        &store,
                        &run_id,
                        expected_generation,
                        serde_json::json!({"event":"host_command_registration_pause", "call_id":call_id,
                            "command_id":command.command_id, "evidence":evidence, "resume":resume}),
                    )?;
                    if let Err(error) = event {
                        tracing::warn!(%error, "host command I/O pause event not recorded");
                    }
                    return Err(WorkflowError::ControlPaused(message));
                }
                Err(error) => return Err(secrets.error(error)),
            };
            let observed =
                secrets.seal_process_output(observed, anchor, pause, &command.command_id)?;
            let Some(kind) = observed.kind else {
                return Ok(observed);
            };
            history.push(OperationalAttempt {
                attempt,
                reason: kind.label(),
                elapsed_secs: started.elapsed().as_secs(),
                progress: observed.progress,
            });
            let report = OperationalReport {
                run_id: &run_id,
                call_id,
                command_id: &command.command_id,
                limit_secs: command.timeout_secs,
                attempts: &history,
            };
            // A stalled teardown kept its record: processes it started may
            // still write the call's roots, so no retry runs beside them.
            // The run pauses, and a resume refuses until they are gone.
            let stalled =
                crate::command::workflow_host_command_groups::stalled_running(&self.run_root)
                    .map_err(|error| format!("the host command records cannot be read: {error}"))
                    .and_then(|records| {
                        (!records.is_empty())
                            .then(|| {
                                records
                                    .iter()
                                    .map(|record| {
                                        format!(
                                    "host command '{}' (process group {}){}",
                                    record.command_id,
                                    record.pgid,
                                    crate::command::workflow_host_command_groups::stall_note(
                                        record
                                    )
                                )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            })
                            .map_or(Ok(()), Err)
                    });
            if let Err(evidence) = stalled {
                return Err(pause_for_stall(
                    &store,
                    &self.run_root,
                    expected_generation,
                    &report,
                    &secrets.text(&evidence),
                ));
            }
            // A child that read nothing over a journal no read could settle
            // already retried the settlement: pause now (Issue 338).
            if let Some(evidence) = &observed.unsettled_publish {
                return Err(pause_for_unsettled_publish(
                    &store,
                    &self.run_root,
                    expected_generation,
                    &report,
                    &secrets.text(&evidence),
                ));
            }
            match next_step(&history) {
                NextStep::Retry => record_retry(&store, &self.run_root, &report),
                NextStep::Pause(cause) => {
                    return Err(pause_run(
                        &store,
                        &self.run_root,
                        expected_generation,
                        &report,
                        cause,
                    ));
                }
            }
        }
    }
}
