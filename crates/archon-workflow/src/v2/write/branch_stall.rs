//! A write branch that stops making progress pauses its run (Issue 263).
//!
//! A size re-ask that no longer shrinks the overshoot, transport drops with
//! no progress between them, and the runner's own no-progress stop are
//! stalls. A stall is never the end of the branch's work: the run is paused
//! with the evidence (`pause_with_evidence`), owned by the generation that
//! prepared the branch, and the branch runs again on resume. When that
//! generation no longer owns the run, nothing is paused and the obsolete
//! branch stops with the run's own control decision. A branch with no run to
//! pause (`pause` unset) keeps reporting its outcome as before.

use super::WorktreeBranchExecution;
use crate::WorkflowError;

/// The control error a stalled `branch` ends with, after pausing its run;
/// `None` when it has no run, or the pause could not be recorded at all.
pub(super) fn pause(
    branch: &WorktreeBranchExecution,
    cause: &'static str,
    evidence: &str,
) -> Option<WorkflowError> {
    let run = branch.pause.as_ref()?;
    let resume = format!("archon workflow resume --live --yes {}", run.run_id);
    let message = format!(
        "write branch '{}' stopped making progress ({cause}): {evidence}; the run is paused, not failed, and the branch runs again on resume: {resume}",
        branch.id
    );
    let detail = serde_json::json!({
        "event": "write_branch_stall_pause",
        "branch": branch.id,
        "call_id": branch.execution.call.id,
        "cause": cause,
        "evidence": evidence,
        "resume": resume,
    });
    match crate::control_pause::pause_with_evidence(&run.store, &run.run_id, run.generation, detail)
    {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, "write branch stall pause event not recorded");
            }
            tracing::warn!(run_id = %run.run_id, "{message}");
            Some(WorkflowError::ControlPaused(message))
        }
        Err(stop @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))) => {
            Some(stop)
        }
        Err(error) => {
            tracing::warn!(%error, "write branch stall pause could not be recorded");
            None
        }
    }
}
