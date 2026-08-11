//! The task-board item a workflow stage branch holds while it runs.
//!
//! `raise_delegated_task` had exactly two callers — the `TaskCreate` tool and
//! the `Agent` tool — and a workflow stage goes through neither. The board was
//! reachable for the whole run (`workflow_live_board.rs` installs the handle,
//! and the lifecycle's drain gate reads the partition) and nothing ever wrote to
//! it, so a run that dispatched seven stages showed an empty board for hours
//! while the half of the system that does the actual work was invisible (#161).
//!
//! Three things here are deliberate.
//!
//! **Partitioning is free and is not re-invented.** A stage session id is
//! `{run_id}-stage-{stage}-attempt-{n}` and
//! [`run_id_for_session`](archon_tools::board::run_id_for_session) splits on the
//! first `-stage-`, so handing the stage session id straight to
//! `raise_delegated_task` lands the item on the `wf-…` run's own partition —
//! one board per run, one item per stage branch, no new id scheme.
//!
//! **A branch is a note, not an issue.** The lifecycle's drain gate refuses a
//! run that still owns an unresolved *issue*, and a branch closes `in_review`
//! by design, so mirroring branches as issues makes every run block on itself —
//! measured, not guessed: it turned the full-lifecycle fixture's `Accepted` into
//! `NeedsReview` with `blocked-board-drain`. See
//! [`raise_delegated_branch`](archon_tools::board::raise_delegated_branch).
//!
//! **The guard closes on `Drop`.** Both dispatch paths that use it have several
//! exits — every required activity emit is a `?`, and a required emit failing is
//! precisely the case where a hand-written close at the bottom would never run.
//! An item left `claimed` by a stage that has already unwound is worse than no
//! item: it reads as live work forever.
//!
//! **Every board failure is soft.** `raise_delegated_task` returns `None` when
//! the board is unreachable and `close_delegated_task` swallows its errors, so a
//! stage dispatches and completes identically whether or not a board exists —
//! which is what makes it safe to run this on the hot path of every branch.

use archon_tools::board::{DelegatedOutcome, close_delegated_task, raise_delegated_branch};
use archon_workflow::{StageRunRequest, WorkflowError};

/// A raised-and-claimed board item, closed exactly once when the stage unwinds.
///
/// Held by value for the life of the branch. [`finish`](Self::finish) records
/// the verdict; the close itself happens in [`Drop`], so an exit that never
/// reaches a `finish` still closes the item rather than leaking it.
pub(crate) struct StageBoardItem {
    /// `None` when the board was unreachable — the run carries on regardless.
    item_id: Option<String>,
    /// What this item closes as. Failure by default: a branch that unwound
    /// without recording a verdict did not do its work, and `escalated` puts
    /// that in front of a human instead of quietly resolving it.
    outcome: DelegatedOutcome,
}

impl StageBoardItem {
    /// Put this stage branch on its run's board, already claimed.
    ///
    /// `instruction` is the prompt the branch was actually handed, not a
    /// paraphrase of it: the board's evidence field is what lets a later reader
    /// see what was really asked.
    pub(crate) fn raise(
        request: &StageRunRequest,
        session_id: &str,
        ordinal: usize,
        agent_name: &str,
        instruction: &str,
    ) -> Self {
        let item_id = raise_delegated_branch(
            session_id,
            &stage_subagent_id(session_id, ordinal, agent_name),
            &format!("[{}] {}", request.stage_id, request.task),
            instruction,
            &raised_by(request),
        );
        Self {
            item_id,
            // Overwritten by `finish` on every exit that reaches one.
            outcome: DelegatedOutcome::Failed,
        }
    }

    /// Record how the branch ended. The write happens on drop.
    pub(crate) fn finish(&mut self, outcome: DelegatedOutcome) {
        self.outcome = outcome;
    }
}

impl Drop for StageBoardItem {
    fn drop(&mut self) {
        if let Some(item_id) = &self.item_id {
            close_delegated_task(item_id, self.outcome);
        }
    }
}

/// The id the branch's agent actually executes under, used as the item id.
///
/// There is no subagent uuid on this path to reuse the way `TaskCreate` reuses
/// one, so the identity has to be derived — and the derivation is not free
/// choice. `archon-pipeline`'s subagent adapter mints
/// `{session}-{ordinal}-{agent}` from the same three values this dispatch is
/// built from and registers *that* id for liveness
/// (`crates/archon-pipeline/src/subagent_adapter.rs`). Using it as the board
/// item id is what makes the claim a real lease: `release_dead_claims` polls
/// `claimed_by`, so an item claimed under an id no registry has heard of would
/// be swept back to `open` the first time anything read the run's board —
/// mid-stage — and the close, a compare-and-set from `claimed`, would then find
/// nothing to close.
///
/// It is unique per stage attempt for free, because the session id it is built
/// from already carries the stage and the attempt, and it is recomputed rather
/// than stored, so nothing has to be threaded to the close.
fn stage_subagent_id(session_id: &str, ordinal: usize, agent_name: &str) -> String {
    format!("{session_id}-{ordinal}-{agent_name}")
}

/// Who raised the item. The run did — no agent asked for this branch.
fn raised_by(request: &StageRunRequest) -> String {
    format!("workflow:{}", request.run_id)
}

/// How a failed stage should close.
///
/// The default is escalation, because a stage that failed is work somebody has
/// to decide about. Two cases are not that:
///
/// - run control stopped the stage (`ControlCancelled`/`ControlPaused`) — the
///   work was never refused or attempted-and-failed, and a paused run may run
///   this very branch again;
/// - the branch's subagent was cancelled, which reaches here as the adapter's
///   message inside a `StageFailed`. Read out of the message for the same
///   reason `llm_retry::transient_live_agent_error_for_request` reads the same
///   sentence out of the same string: cancellation is reported as a transport
///   failure, and this is the only place the distinction still exists.
///
/// A *timeout* is deliberately not in that list. The branch was given the work
/// and did not finish it, which is a failure somebody should see.
pub(crate) fn stage_board_outcome(error: &WorkflowError) -> DelegatedOutcome {
    match error {
        WorkflowError::ControlCancelled(_) | WorkflowError::ControlPaused(_) => {
            DelegatedOutcome::Stopped
        }
        WorkflowError::StageFailed(message)
            if message.to_ascii_lowercase().contains("subagent cancelled") =>
        {
            DelegatedOutcome::Stopped
        }
        _ => DelegatedOutcome::Failed,
    }
}
