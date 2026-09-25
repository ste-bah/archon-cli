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
//! The re-ask runs under what the first attempt left of the branch timeout,
//! but never under less than half of it ([`reask_timeout_secs`]). A review map
//! branch that spent its whole wall clock used to be given a second full one,
//! doubling the worst case of the map; a zero budget instead would make the
//! re-ask pointless, since a read-only branch carries nothing forward. So the
//! attempts of one branch together run at most one and a half timeouts.
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

/// The wall clock, in seconds, the host's one re-ask runs under: what the
/// first attempt left of `timeout`, and never less than half of it. `None`
/// when the branch has no timeout to cap.
pub(super) fn reask_timeout_secs(timeout: Option<u64>, first_elapsed_secs: u64) -> Option<u64> {
    let timeout = timeout?;
    Some(
        timeout
            .saturating_sub(first_elapsed_secs)
            .max(timeout.div_ceil(2))
            .max(1),
    )
}

/// Run one branch's attempts under both budgets. `attempt` dispatches the
/// branch once, under the given wall clock in seconds when one is given and
/// under the branch's own `timeout_secs` otherwise; `on_reask` is told before
/// the host's one re-ask is spent. Transport re-asks after the host's re-ask
/// keep its capped wall clock.
pub(super) async fn with_host_retry<F, Fut>(
    review_map: bool,
    timeout_secs: Option<u64>,
    on_reask: &(dyn Fn(HostReask, &str) + Sync),
    mut attempt: F,
) -> WorkflowResult<WorkflowV2Result>
where
    F: FnMut(Option<u64>) -> Fut,
    Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
{
    let mut transport_failures = 0usize;
    let mut reasked: Option<HostReask> = None;
    let mut budget: Option<u64> = None;
    loop {
        let started = std::time::Instant::now();
        let error = match attempt(budget).await {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        match next_step(&error, review_map, transport_failures, reasked.is_some()) {
            NextStep::Transport => transport_failures += 1,
            NextStep::Reask(reason) => {
                on_reask(reason, &error.to_string());
                reasked = Some(reason);
                budget = reask_timeout_secs(timeout_secs, started.elapsed().as_secs());
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
