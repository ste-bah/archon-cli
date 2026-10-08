//! Issue 364: a provider silent for a whole no-progress window pauses the run.
//!
//! A session's stream round resends its request, with backoff, for as long
//! as its provider answers within a no-progress window (a network that needs
//! a while to come back after a wake). When the provider stays silent for
//! that whole window, the session ends with the transport-stall marker.
//! Nothing was wrong with the work and an immediate re-ask cannot help, so
//! the call's dispatch pauses the run with the evidence, the same transition
//! `workflow pause` makes. A resume runs the call again; the run never fails
//! on it.

use super::*;

impl WorkflowScriptHost {
    /// `error` unchanged unless it is a transport stall the run can pause on;
    /// then the pause, as the control error the dispatch ends with.
    pub(super) async fn pause_on_transport_stall(
        &self,
        call_id: &str,
        generation: Option<u64>,
        error: WorkflowError,
    ) -> WorkflowError {
        if !archon_workflow::error::is_transport_stall(&error)
            // After a terminal host stop (#285) that stop is the outcome.
            || self.accumulator.lock().await.terminal_host_stop
        {
            return error;
        }
        // No generation known: nothing can be paused; the error stands.
        let Some(generation) = generation else {
            return error;
        };
        let (store, run_id) = (&self.runner.workflow_store, &self.runner.run_id);
        let resume = format!("archon workflow resume --live --yes {run_id}");
        let message = format!(
            "call `{call_id}`: the provider gave no answer for a whole no-progress window of resends: {error}; the run is paused, not failed: when the provider is reachable again, {resume}"
        );
        let detail = serde_json::json!({
            "event": "transport_stall_pause",
            "cause": "transport_stall",
            "call_id": call_id,
            "error": crate::command::workflow_decompose_events::bounded_log_field(&error.to_string()),
            "resume": resume,
        });
        match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail)
        {
            Ok(event) => {
                if let Err(error) = event {
                    tracing::warn!(%error, run_id, "transport stall pause event not recorded");
                }
                tracing::warn!(run_id, "{message}");
                WorkflowError::ControlPaused(message)
            }
            Err(
                paused @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)),
            ) => paused,
            Err(failure) => {
                tracing::warn!(%failure, run_id, "the transport stall pause could not be recorded");
                error
            }
        }
    }
}
