//! Deliberate script stops are host control flow, never JavaScript error text.
//! Only the privileged fixed-script host accepts this versioned request. Its
//! owned, recorded stop is sticky even if JS catches or rewrites the rejection;
//! throwing an identical object or message cannot create that host evidence.

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
            let detail = serde_json::json!({
                "event": "script_terminal_stop",
                "schema_version": request.schema_version,
                "reason": reason,
                "generation": run.generation,
                "call_id": "workflow.js",
                "status": WorkflowV2Status::Failed,
            });
            WorkflowEventLog::new(locked.clone()).emit(
                &run.id,
                locked.next_event_seq(&run.id)?,
                WorkflowEventKind::StageFailed,
                detail,
            )?;
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
