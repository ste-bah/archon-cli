//! The fixed executor's answer to an operational ending (Issue #255): retry
//! the same call while it makes progress, else pause the run. The contract
//! and the policy live in `workflow_host_command_operational`.
use super::*;
use crate::command::workflow_host_command_operational::{
    NextStep, OperationalAttempt, OperationalReport, classify, next_step, pause_for_stall,
    pause_run, record_retry, reported_progress, require_run_owned,
};

impl FixedHostCommandExecutor {
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
    ) -> WorkflowResult<SupervisedProcessOutput> {
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
        loop {
            let attempt = history.len() as u32 + 1;
            if attempt > 1 {
                require_run_owned(&store, &run_id, expected_generation)?;
                // The killed attempt's partial staging must not reach the
                // audit, which requires the staged set to match the manifest.
                prepare_staging(&self.run_root, call_id)
                    .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            }
            let (control, handle) = HostCommandControl::new();
            let started = std::time::Instant::now();
            let observed = self
                .execute_process_with_run_control(
                    command.clone(),
                    control,
                    handle,
                    expected_generation,
                )
                .await?;
            let Some(kind) = classify(&observed) else {
                return Ok(observed);
            };
            history.push(OperationalAttempt {
                attempt,
                reason: kind.label(),
                elapsed_secs: started.elapsed().as_secs(),
                progress: reported_progress(&observed.stderr),
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
                    &evidence,
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
