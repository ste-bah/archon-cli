//! Issue 335: a workflow.js runtime error pauses the run with its evidence.
//!
//! The workflow self-heals; a crash of the script is not the end of the
//! work it has recorded. A script that throws (or rejects) while it runs
//! PAUSES the run, with the JavaScript error text as evidence, the same
//! transition `workflow pause` makes. A resume runs the script again from
//! the top, and every call it recorded complete answers from its record, so
//! only the work after the crash runs again.
//!
//! The authoring bootstrap keeps the failure so its caller can re-author.
//! Outside the fixed host, a source that cannot be evaluated (a syntax or
//! top-level error, before the script's promise exists and before any call) FAILS the
//! run: a resume evaluates the same source and cannot change it. The caller
//! makes that distinction. The fixed host pauses these errors too; only a
//! validated terminal-stop request ends its script terminally.
//!
//! A deterministic crash recurs on resume. Each pause records the error and
//! the point it stopped at (the calls answered so far, and the last one) in
//! [`SCRIPT_ERROR_PAUSE_RECORD`]. When a resume stops with the same error at
//! the same point, the pause says so: it counts the recurrences and tells
//! the operator that a resume alone re-runs it unchanged. The run stays
//! paused (never failed, never resumed by itself), so nothing loops.
//!
//! Issue 337: a fixed script's pause here (and the fixed run boundary's)
//! records the attempts it covers exactly as a `w.pause` does
//! (`HostPauseCoverage`), so a resume replays every recorded answer of that
//! script verbatim -- refusals and failed calls too -- and only the work
//! after the crash runs again.

use super::*;

/// The last script-error pause of the run, relative to its run directory.
pub(in super::super) const SCRIPT_ERROR_PAUSE_RECORD: &str = "v2/script-error-pause.json";

/// Where the script stopped: the calls the host answered (executed or
/// reused), and the last of them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StopPoint {
    calls: usize,
    last_call: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ScriptErrorPauseRecord {
    script_error: String,
    point: StopPoint,
    occurrences: u64,
}

impl WorkflowScriptHost {
    /// The authoring bootstrap re-authors on error. Fixed scripts pause on
    /// every unmarked error; deliberate stops are validated by the host API.
    pub(in super::super) fn pauses_on_script_errors(&self) -> bool {
        self.runner.pauses_on_script_error
    }

    pub(in super::super) async fn finish_script_error(
        &self,
        error: &str,
        unevaluable: bool,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        if unevaluable && !self.runner.raw_outcomes_allowed {
            let error = format!(
                "workflow.js cannot be evaluated (a syntax or top-level error, before any call); a resume evaluates the same source and cannot change it, so fix the script source: {error}"
            );
            return Ok(self.mark_script_failure(&error).await);
        }
        if self.pauses_on_script_errors() {
            if let Some(stop) = self.pause_on_script_error(error).await {
                return Err(stop);
            }
            if self.runner.raw_outcomes_allowed {
                return Err(WorkflowError::SpecInvalid(format!(
                    "workflow.js error: {error}; the host could not persist its script-error pause"
                )));
            }
        }
        Ok(self.mark_script_failure(error).await)
    }

    /// Pauses the run on the runtime error `error` of its workflow.js and
    /// returns the pause as the error the session ends with. A session that
    /// no longer owns the run changes nothing and gets the run's own control
    /// decision. `None` when nothing could be recorded: the fixed caller
    /// reports the persistence fault; other callers keep their prior policy.
    pub(in super::super) async fn pause_on_script_error(
        &self,
        error: &str,
    ) -> Option<WorkflowError> {
        self.pause_on_script_stop(error, "script_error").await
    }

    /// [`Self::pause_on_script_error`] for any stop of the script the host
    /// decided (Issue 364: a starved script thread); `cause` names it in the
    /// pause event while it does not recur.
    pub(in super::super) async fn pause_on_script_stop(
        &self,
        error: &str,
        cause: &str,
    ) -> Option<WorkflowError> {
        let script_error = crate::command::workflow_decompose_events::bounded_log_field(error);
        let point = {
            let acc = self.accumulator.lock().await;
            StopPoint {
                calls: acc.calls.len(),
                last_call: acc.calls.last().map(|call| call.id.clone()),
            }
        };
        let (store, run_id) = (&self.runner.workflow_store, &self.runner.run_id);
        let run = match store.load_state(run_id) {
            Ok(run) => run,
            Err(unreadable) => {
                tracing::warn!(%unreadable, run_id, "script error pause: run state unreadable");
                return None;
            }
        };
        // A stale session (a resume gave the run to a newer executor)
        // pauses nothing.
        if let Err(refused) = self.runner.v2_store.require_session_executor(&run) {
            return Some(refused);
        }
        // Issue 337: a fixed script's crash pause covers what the run
        // recorded, as a `w.pause` does, so a resume replays a judge's
        // refusal or a failed author call verbatim instead of re-asking it.
        // A v3 script keeps its Issue 335 contract: failed calls run again.
        let coverage = self.runner.raw_outcomes_allowed.then(|| {
            HostPauseCoverage::snapshot(
                &self.runner.v2_store,
                self.runner.host_command_executor.as_ref(),
            )
        });
        // An unreadable record is evidence lost, not state: counted afresh.
        let prior = std::fs::read(store.run_dir(run_id).join(SCRIPT_ERROR_PAUSE_RECORD))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ScriptErrorPauseRecord>(&bytes).ok());
        let occurrences = match prior {
            Some(prior) if prior.script_error == script_error && prior.point == point => {
                prior.occurrences.saturating_add(1)
            }
            _ => 1,
        };
        let resume = format!("archon workflow resume --live --yes {run_id}");
        let at = point.last_call.as_deref().map_or_else(
            || "before its first call".to_string(),
            |id| format!("after call `{id}`"),
        );
        let recurring = occurrences > 1;
        let remedy = if recurring {
            format!(
                "the same error stopped the script at the same point on {occurrences} executions in a row, so a resume alone re-runs it unchanged: fix what the error names (the workflow script or the state it reads), then {resume}"
            )
        } else {
            format!(
                "{resume} re-runs the script and reuses its {} recorded call(s); if the error is in the script or the state it reads, fix that first",
                point.calls
            )
        };
        let detail = serde_json::json!({
            "event": "script_error_pause",
            "cause": if recurring { "recurring_script_error" } else { cause },
            "call_id": "workflow.js",
            "script_error": script_error,
            "calls_answered": point.calls,
            "last_call": point.last_call,
            "occurrences": occurrences,
            "record_path": SCRIPT_ERROR_PAUSE_RECORD,
            "resume": resume,
        });
        // The coverage is written in the pause's own lock section, so no
        // resume can take the run before its replay record exists.
        match archon_workflow::control_pause::pause_owned_then(
            store,
            run_id,
            archon_workflow::control_pause::PauseOwner::Generation(run.generation),
            detail,
            |locked, seq| {
                if let Some(coverage) = coverage {
                    coverage.record(locked, run_id, "script-error", seq);
                }
            },
        ) {
            Ok(event) => {
                if let Err(error) = event {
                    tracing::warn!(%error, run_id, "script error pause event not recorded");
                }
            }
            Err(
                control @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)),
            ) => {
                return Some(control);
            }
            Err(failure) => {
                tracing::warn!(%failure, run_id, "the script error pause could not be recorded");
                return None;
            }
        }
        let record = ScriptErrorPauseRecord {
            script_error: script_error.clone(),
            point,
            occurrences,
        };
        // Losing it costs only the recurrence count of a later pause.
        if let Err(error) = store.write_run_json(run_id, SCRIPT_ERROR_PAUSE_RECORD, &record) {
            tracing::warn!(%error, run_id, "script error pause record not written");
        }
        let message = format!(
            "workflow.js stopped with a runtime error {at}: {script_error}; run {run_id} is paused, not failed. Next: {remedy}"
        );
        tracing::warn!(run_id, "{message}");
        Some(WorkflowError::ControlPaused(message))
    }
}
