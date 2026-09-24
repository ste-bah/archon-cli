//! The two host bounds on one agent session, raced against its run: the wall
//! clock, and inactivity (`archon_tools::subagent_activity`).
//!
//! Each bound cancels the session the same way and then waits for it to wind
//! down; what differs is how the ending is reported. A wall-clock cut keeps the
//! "subagent timed out after Ns" text every host classifier already knows. An
//! inactivity cut carries its own marker, so a record says which bound fired —
//! a session that was slow and one that had stopped look nothing alike.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use archon_tools::subagent_activity::{ActivityClock, inactivity_error_text, silence_exceeding};
use archon_tools::subagent_executor::SubagentOutcome;
use tokio_util::sync::CancellationToken;

pub(super) type SessionRun = Pin<Box<dyn Future<Output = SubagentOutcome> + Send>>;

/// Which host bound ended a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostCut {
    WallClock,
    Inactivity { silent: Duration, limit: Duration },
}

/// The inactivity bound for one session: the clock its runner reports to and
/// the silence that ends it.
pub(super) struct InactivityBound {
    pub clock: Arc<ActivityClock>,
    pub limit: Duration,
}

impl InactivityBound {
    pub fn new(limit: Option<Duration>) -> Option<Self> {
        limit.map(|limit| Self {
            clock: ActivityClock::new(),
            limit,
        })
    }

    /// Install the clock for the runner of session `agent_id` to report to.
    pub fn install(&self, agent_id: &str, run: SessionRun) -> SessionRun {
        Box::pin(archon_tools::subagent_activity::scope(
            archon_tools::subagent_activity::SessionClock {
                agent_id: agent_id.to_string(),
                clock: Arc::clone(&self.clock),
            },
            run,
        ))
    }
}

async fn wall_clock(timeout_secs: Option<u64>) {
    match timeout_secs {
        Some(secs) => tokio::time::sleep(Duration::from_secs(secs.max(1))).await,
        None => std::future::pending().await,
    }
}

async fn inactivity(bound: Option<&InactivityBound>) -> (Duration, Duration) {
    match bound {
        Some(bound) => (
            silence_exceeding(&bound.clock, bound.limit).await,
            bound.limit,
        ),
        None => std::future::pending().await,
    }
}

/// Run the session to its end, or cut it at whichever host bound fires first.
pub(super) async fn drive(
    mut run: SessionRun,
    cancel: &CancellationToken,
    timeout_secs: Option<u64>,
    bound: Option<&InactivityBound>,
) -> (SubagentOutcome, Option<HostCut>) {
    let cut = tokio::select! {
        outcome = &mut run => return (outcome, None),
        _ = wall_clock(timeout_secs) => HostCut::WallClock,
        (silent, limit) = inactivity(bound) => HostCut::Inactivity { silent, limit },
    };
    cancel.cancel();
    (run.await, Some(cut))
}

/// The error an inactivity cut ends the call with, or `None` when the session
/// was not cut for inactivity or finished before the cancellation reached it.
pub(super) fn inactivity_failure(
    outcome: &SubagentOutcome,
    cut: Option<HostCut>,
) -> Option<anyhow::Error> {
    let Some(HostCut::Inactivity { silent, limit }) = cut else {
        return None;
    };
    match outcome {
        SubagentOutcome::Completed(_) => None,
        _ => Some(anyhow!("{}", inactivity_error_text(silent, limit))),
    }
}

#[cfg(test)]
#[path = "host_cuts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "host_cuts_session_tests.rs"]
mod session_tests;
