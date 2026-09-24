//! How often the host re-asks a read-only branch, and why.
//!
//! Two budgets, never pooled:
//!
//! - **Transport**: a dropped provider connection is not a verdict on the
//!   work, so the branch is re-asked up to `MAX_TRANSPORT_RETRIES` times.
//! - **One host re-ask**: a branch the host itself cut for INACTIVITY, and any
//!   REVIEW MAP branch that failed for any reason, is re-asked exactly once.
//!   An inactivity cut is typically one stuck request; a review map branch
//!   that fails leaves its task with no verdict, so it gets a second chance
//!   before the map is done. The re-ask is reported through `on_reask` (a
//!   `branch_reasked` event carrying the first attempt's error), and a branch
//!   that fails again says so in its own error, so a record always shows it.
//!
//! The re-asked attempt stands on its own: a success is judged exactly as any
//! first attempt is (nothing is added to its evidence), and a failure keeps
//! its own text — the first attempt's error is never copied into it, because
//! every classifier downstream reads error text and must classify the failure
//! that actually ended the branch.
//!
//! Control signals (pause/cancel) and content rejections are never re-asked.

use std::future::Future;

use archon_workflow::v2::{WorkflowV2Result, transport_retry};
use archon_workflow::{WorkflowError, WorkflowResult};

/// Why the host re-asked a branch once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostReask {
    Inactivity,
    ReviewIncomplete,
}

impl HostReask {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Inactivity => "inactivity",
            Self::ReviewIncomplete => "review_incomplete",
        }
    }
}

/// What to do with one failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NextStep {
    Stop,
    Transport,
    Reask(HostReask),
}

/// The decision for a failed attempt, given what has already been spent.
pub(super) fn next_step(
    error: &WorkflowError,
    review_map: bool,
    transport_failures: usize,
    reasked: bool,
) -> NextStep {
    if matches!(
        error,
        WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
    ) {
        return NextStep::Stop;
    }
    let text = error.to_string();
    if transport_retry::is_content_rejection(&text) && !review_map {
        return NextStep::Stop;
    }
    if transport_retry::is_transport_failure(&text)
        && !transport_retry::is_content_rejection(&text)
        && transport_failures < transport_retry::MAX_TRANSPORT_RETRIES
    {
        return NextStep::Transport;
    }
    if reasked {
        return NextStep::Stop;
    }
    if archon_workflow::error::is_inactivity_timeout_text(&text) {
        return NextStep::Reask(HostReask::Inactivity);
    }
    if review_map {
        return NextStep::Reask(HostReask::ReviewIncomplete);
    }
    NextStep::Stop
}

/// Marker-free on purpose: no host-cut, transport, timeout or cancellation
/// phrase, so it can never change how the error it is appended to classifies.
fn reask_note(reason: HostReask) -> String {
    format!(
        "[host re-ask spent: this was the second attempt; the first ended as {}]",
        reason.label()
    )
}

/// The final failure of a re-asked branch, keeping its type and its own text
/// so every classifier above reads it exactly as it would a first failure. A
/// variant that carries no free text is returned as it is; the branch event
/// still records the re-ask.
pub(super) fn after_reask(error: WorkflowError, reason: HostReask) -> WorkflowError {
    let note = reask_note(reason);
    match error {
        WorkflowError::HostCallTimeout(text) => {
            WorkflowError::HostCallTimeout(format!("{text} {note}"))
        }
        WorkflowError::StageFailed(text) => WorkflowError::StageFailed(format!("{text} {note}")),
        other => other,
    }
}

/// Run one branch's attempts under both budgets. `attempt` dispatches the
/// branch once; `on_reask` is told before the host's one re-ask is spent.
pub(super) async fn with_host_retry<F, Fut>(
    review_map: bool,
    on_reask: &(dyn Fn(HostReask, &str) + Sync),
    mut attempt: F,
) -> WorkflowResult<WorkflowV2Result>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
{
    let mut transport_failures = 0usize;
    let mut reasked: Option<HostReask> = None;
    loop {
        let error = match attempt().await {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        match next_step(&error, review_map, transport_failures, reasked.is_some()) {
            NextStep::Transport => transport_failures += 1,
            NextStep::Reask(reason) => {
                on_reask(reason, &error.to_string());
                reasked = Some(reason);
            }
            NextStep::Stop => {
                return Err(match reasked {
                    Some(reason) => after_reask(error, reason),
                    None => error,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_read_only_retry_tests.rs"]
mod tests;
