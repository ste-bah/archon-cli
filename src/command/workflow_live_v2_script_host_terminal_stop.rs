//! Deliberate script stops are host control flow, never JavaScript error text.
//! Only the privileged fixed-script host accepts this versioned request. Its
//! owned, recorded stop is sticky even if JS catches or rewrites the rejection;
//! throwing an identical object or message cannot create that host evidence.
//! Issue 337: every host terminal stop of a fixed script -- this request, and
//! a call that ends the script (an unsatisfied final report or human gate) --
//! is persisted the same way, under the run lock ([`persist_terminal_stop`]):
//! the stop record, so a sibling call still in flight can neither pause the
//! run nor publish into it and its supervisor ends it; and the coverage of
//! the verdicts the run holds, so when the finalization is lost (the process
//! dies, or finalizing fails) and stale-owner recovery pauses the run, a
//! resume replays the verdict that decided the stop and reaches the same
//! stop, instead of asking the judge again.

use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalStopRequest {
    schema_version: u32,
    reason: String,
}

impl WorkflowScriptHost {
    pub(super) async fn request_script_terminal_stop(
        &self,
        payload: &str,
    ) -> archon_workflow::WorkflowResult<String> {
        if !self.runner.raw_outcomes_allowed {
            return Err(WorkflowError::PolicyDenied(
                "terminalStop requires the trusted fixed-script host".into(),
            ));
        }
        if payload.len() > 65536 {
            return Err(WorkflowError::SpecInvalid(
                "terminalStop payload is too large".into(),
            ));
        }
        let request: TerminalStopRequest = serde_json::from_str(payload)?;
        if request.schema_version != 1
            || request.reason.trim().is_empty()
            || request.reason.len() > 16384
        {
            return Err(WorkflowError::SpecInvalid(
                "terminalStop requires schemaVersion=1 and a non-empty reason of at most 16384 bytes".into(),
            ));
        }
        let reason = crate::command::workflow_decompose_events::bounded_log_field(&request.reason);
        let mut acc = self.accumulator.lock().await;
        // The lock and executor check make an obsolete session incapable of
        // authorizing a stop of the run now owned by a resume.
        self.with_owned_run_lock(|locked| {
            let run = locked.load_state(&self.runner.run_id)?;
            match run.status {
                archon_workflow::RunStatus::Paused => {
                    return Err(WorkflowError::ControlPaused(
                        "terminal stop refused: run is paused".into(),
                    ));
                }
                archon_workflow::RunStatus::Cancelled => {
                    return Err(WorkflowError::ControlCancelled(
                        "terminal stop refused: run is cancelled".into(),
                    ));
                }
                _ => {}
            }
            // Issue 337: recorded durably first, under the lock every
            // sibling's pause and publication take, so from here no call of
            // this generation can pause the run or publish into it.
            self.persist_terminal_stop(locked, &run, &reason)?;
            let detail = serde_json::json!({
                "event": "script_terminal_stop",
                "schema_version": request.schema_version,
                "reason": reason,
                "generation": run.generation,
                "call_id": "workflow.js",
                "status": WorkflowV2Status::Failed,
            });
            // Evidence only: the record above is the authority.
            if let Err(error) = locked.next_event_seq(&run.id).and_then(|seq| {
                WorkflowEventLog::new(locked.clone()).emit(
                    &run.id,
                    seq,
                    WorkflowEventKind::StageFailed,
                    detail,
                )
            }) {
                tracing::warn!(%error, run_id = %run.id, "terminal stop event not recorded");
            }
            // Only this host-validated side effect authorizes terminal script
            // failure. An error's shape, code, name and text are irrelevant.
            acc.terminal_host_stop = true;
            acc.status = WorkflowV2Status::Failed;
            acc.failed_call = Some("workflow.js".into());
            acc.failed_result_path = None;
            acc.next_action =
                Some("repair the reported gate refusal, then start a fresh workflow".into());
            acc.script_error = Some(reason.clone());
            Ok(())
        })?;
        Err(WorkflowError::TerminalHostCall(reason))
    }
}

impl WorkflowScriptHost {
    /// The stop a terminal call's record makes (an unsatisfied final report
    /// or human gate): recorded (`mark_terminal`), reported, and returned as
    /// the error that unwinds the script. Issue 337: the same whether the
    /// call ran now or its covered verdict is replayed on a resume, so a
    /// replayed stop never lets the script go on past it.
    pub(in super::super) async fn stop_on_terminal_call(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowError {
        let path = self.runner.v2_store.result_path(&record.call.id);
        let next_action = next_action_for_terminal_call(&record.call.id, record.status);
        self.mark_terminal(record, path.display().to_string(), next_action.clone())
            .await;
        self.emit_v2_event(
            if record.status == WorkflowV2Status::Failed {
                WorkflowEventKind::StageFailed
            } else {
                WorkflowEventKind::StageStalled
            },
            serde_json::json!({
                "event": "script_stopped",
                "call_id": record.call.id.clone(),
                "method": record.call.method.as_str(),
                "status": record.status,
                "result_path": path.display().to_string(),
                "next_action": next_action,
            }),
        );
        WorkflowError::TerminalHostCall(format!(
            "{} ended with {:?}",
            record.call.id, record.status
        ))
    }

    /// Persists a terminal stop of `run` (its lock held, ownership checked):
    /// the coverage snapshot, then the stop record. The record is the
    /// authority; the coverage is replay evidence and never fails the stop.
    fn persist_terminal_stop(
        &self,
        locked: &WorkflowStore,
        run: &archon_workflow::WorkflowRun,
        reason: &str,
    ) -> archon_workflow::WorkflowResult<()> {
        let coverage = HostPauseCoverage::snapshot(
            &self.runner.v2_store,
            self.runner.host_command_executor.as_ref(),
        );
        archon_workflow::control_pause::record_terminal_stop(locked, run, reason)?;
        coverage.record(locked, &run.id, "terminal-stop", None);
        Ok(())
    }

    /// The terminal stop a call's own status made (`mark_terminal`), for the
    /// fixed host only, like `terminalStop`. A stop that cannot be persisted
    /// fails safe, as `terminalStop` does: the run PAUSES with the refusal as
    /// evidence (a stored pause outranks this session's in-memory stop), so
    /// no sibling can end it unrecorded ([`Self::pause_unpersisted_stop`]).
    pub(in super::super) fn persist_call_terminal_stop(&self, record: &WorkflowV2CallRecord) {
        if !self.runner.raw_outcomes_allowed {
            return;
        }
        let reason = format!("{} ended with {:?}", record.call.id, record.status);
        let persisted = self.with_owned_run_lock(|locked| {
            let run = locked.load_state(&self.runner.run_id)?;
            if !matches!(
                run.status,
                archon_workflow::RunStatus::Planned | archon_workflow::RunStatus::Running
            ) {
                return Ok(());
            }
            self.persist_terminal_stop(locked, &run, &reason)
        });
        match persisted {
            Ok(()) => {}
            // The run's own control decision stands: a session a resume
            // replaced changes nothing (Issue 291), a paused run stays so.
            Err(
                control @ (WorkflowError::ControlCancelled(_) | WorkflowError::ControlPaused(_)),
            ) => {
                tracing::warn!(%control, run_id = %self.runner.run_id, "terminal stop not persisted: this session does not own a running run");
            }
            Err(error) => self.pause_unpersisted_stop(record, &error),
        }
    }

    /// Pauses the run whose stop by `record` could not be persisted. The
    /// pause is this session's own: owned by the generation it samples only
    /// while its executor owns the run (`owned_generation`) and fenced again
    /// under the run lock, so a session a resume replaced pauses nothing. It
    /// records the coverage of the verdicts the run holds, as a crash pause
    /// does, so a resume replays the verdict that decided the stop and
    /// reaches the same stop instead of asking it again.
    fn pause_unpersisted_stop(&self, record: &WorkflowV2CallRecord, error: &WorkflowError) {
        let run_id = &self.runner.run_id;
        let generation = match self.owned_generation() {
            Ok(generation) => generation,
            Err(refused) => {
                tracing::warn!(%refused, run_id, "the unpersisted terminal stop pauses nothing");
                return;
            }
        };
        let refusal = format!(
            "the terminal stop of call {} could not be persisted ({error}); run {run_id} is paused, not ended: repair the run store, then archon workflow resume --live --yes {run_id}",
            record.call.id
        );
        tracing::warn!(run_id, "{refusal}");
        let coverage = HostPauseCoverage::snapshot(
            &self.runner.v2_store,
            self.runner.host_command_executor.as_ref(),
        );
        let paused = archon_workflow::control_pause::pause_owned_then(
            &self.runner.workflow_store,
            run_id,
            archon_workflow::control_pause::PauseOwner::Generation(generation),
            serde_json::json!({
                "event": "terminal_stop_unpersisted",
                "call_id": record.call.id,
                "status": record.status,
                "refusal": refusal,
            }),
            |locked, seq| coverage.record(locked, run_id, "terminal-stop-unpersisted", seq),
        );
        if let Err(error) = paused {
            tracing::warn!(%error, run_id, "the unpersisted terminal stop could not pause the run");
        }
    }
}
