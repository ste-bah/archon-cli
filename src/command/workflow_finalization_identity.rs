//! Which finalization calls own the terminal commit, and which are replays.
//!
//! The terminal record's only immutable identity is the run kind: a record
//! carrying a different kind is state from another workflow and is corrupt.
//! The terminal status is not identity. It is the outcome of one execution
//! attempt, and an attempt that did not complete the run leaves the run
//! resumable, so a later attempt may legitimately record a different outcome
//! than the one already on the record. A record with no decided v2 terminal
//! status has never committed a v2 outcome at all — the run-status path (a
//! pause, for instance) leaves that field empty — so deciding one is the
//! normal transition rather than a changed identity.
//!
//! The one outcome that can never be superseded is a completing one: a
//! completed run refuses resume, so nothing may run again and contradict it.

use archon_workflow::{
    FinalizationRecordV1, RunStatus, WorkflowError, WorkflowResult, WorkflowRunKind,
    WorkflowV2Status,
};

/// What a finalization call must do with the terminal record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    /// This call owns the commit: write a record for its own outcome and
    /// commit the terminal state and event once.
    Commit,
    /// The persisted record already carries this outcome: reuse it and never
    /// commit the same outcome twice.
    Replay,
}

/// Disposition of a v2 summary finalization.
///
/// An absent `terminal_v2_status` means no v2 terminal status has ever been
/// decided for this run; this call is the decision, not a replay of it.
pub(super) fn summary_disposition(
    persisted: Option<&FinalizationRecordV1>,
    run_kind: WorkflowRunKind,
    status: WorkflowV2Status,
) -> WorkflowResult<Disposition> {
    let Some(persisted) = persisted else {
        return Ok(Disposition::Commit);
    };
    require_same_run_kind(persisted, run_kind)?;
    if persisted.terminal_v2_status == Some(status) {
        return Ok(Disposition::Replay);
    }
    require_supersedable(persisted, &format!("{status:?}"))?;
    Ok(Disposition::Commit)
}

/// Disposition of a run-status finalization, which never decides a v2 status.
pub(super) fn run_status_disposition(
    persisted: Option<&FinalizationRecordV1>,
    run_kind: WorkflowRunKind,
    status: &RunStatus,
) -> WorkflowResult<Disposition> {
    let Some(persisted) = persisted else {
        return Ok(Disposition::Commit);
    };
    require_same_run_kind(persisted, run_kind)?;
    if persisted.terminal_status == *status {
        return Ok(Disposition::Replay);
    }
    require_supersedable(persisted, &format!("{status:?}"))?;
    Ok(Disposition::Commit)
}

/// The run kind is the record's identity and never changes for one run.
fn require_same_run_kind(
    persisted: &FinalizationRecordV1,
    run_kind: WorkflowRunKind,
) -> WorkflowResult<()> {
    if persisted.run_kind != run_kind {
        return Err(WorkflowError::StateCorrupt(format!(
            "finalization identity changed: persisted run kind {:?}, current {:?}",
            persisted.run_kind, run_kind
        )));
    }
    Ok(())
}

/// A completing outcome is final; anything else is a resume point.
fn require_supersedable(persisted: &FinalizationRecordV1, current: &str) -> WorkflowResult<()> {
    if persisted.is_completing() {
        return Err(WorkflowError::StateCorrupt(format!(
            "finalization contradicts the completed outcome already committed: persisted {:?}/{:?}, current {current}",
            persisted.terminal_status, persisted.terminal_v2_status
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_finalization_identity_tests.rs"]
mod tests;
