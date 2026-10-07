//! The two host bounds on one agent session, raced against its run: the wall
//! clock, and inactivity (`archon_tools::subagent_activity`).
//!
//! The wall clock counts execution only: it starts when the session takes its
//! subagent slot, so a wait for one does not run it down (Issue 288).
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
use archon_tools::subagent_dispatch_clock::{self, DispatchClock, DispatchCut};
use archon_tools::subagent_executor::SubagentOutcome;
use tokio_util::sync::CancellationToken;

pub(super) type SessionRun = Pin<Box<dyn Future<Output = SubagentOutcome> + Send>>;

/// Which host bound ended a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostCut {
    WallClock,
    Inactivity {
        silent: Duration,
        limit: Duration,
    },
    /// The executor reported neither a slot nor a wait for one for a whole
    /// wall clock (Issue 288): named, never timed from dispatch.
    NeverAdmitted {
        limit: Duration,
    },
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

/// The wall clock counts the session's execution only: it starts when the
/// executor reports the session took its slot and does not run while it
/// waits for one (Issue 288), so a queued session is never cut before it has
/// run. A session never admitted is cut with its own diagnosis.
async fn wall_clock(clock: &DispatchClock, timeout_secs: Option<u64>) -> HostCut {
    match timeout_secs {
        Some(secs) => match clock.cut(Duration::from_secs(secs.max(1))).await {
            DispatchCut::Execution(_) => HostCut::WallClock,
            DispatchCut::NeverAdmitted(limit) => HostCut::NeverAdmitted { limit },
        },
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

/// Install the session's own dispatch clock for session `agent_id`, beside
/// the outer deadline clock of the call it serves, if that call has one:
/// both stop while the executor waits for a slot for this session.
pub(super) fn install_dispatch_clock(
    agent_id: &str,
    run: SessionRun,
) -> (Arc<DispatchClock>, SessionRun) {
    let clock = DispatchClock::new();
    let mut clocks = vec![Arc::clone(&clock)];
    clocks.extend(subagent_dispatch_clock::current_call());
    let run = Box::pin(subagent_dispatch_clock::scope_session(
        agent_id.to_string(),
        clocks,
        run,
    ));
    (clock, run)
}

/// Run the session to its end, or cut it at whichever host bound fires first.
/// `clock` is the session's dispatch clock ([`install_dispatch_clock`]).
pub(super) async fn drive(
    mut run: SessionRun,
    cancel: &CancellationToken,
    clock: &DispatchClock,
    timeout_secs: Option<u64>,
    bound: Option<&InactivityBound>,
) -> (SubagentOutcome, Option<HostCut>) {
    let cut = tokio::select! {
        outcome = &mut run => return (outcome, None),
        cut = wall_clock(clock, timeout_secs) => cut,
        (silent, limit) = inactivity(bound) => HostCut::Inactivity { silent, limit },
    };
    cancel.cancel();
    (run.await, Some(cut))
}

/// The error an inactivity cut, or a never-admitted cut, ends the call with;
/// `None` for a wall-clock cut, no cut, or a session that finished before the
/// cancellation reached it.
pub(super) fn inactivity_failure(
    outcome: &SubagentOutcome,
    cut: Option<HostCut>,
) -> Option<anyhow::Error> {
    if let SubagentOutcome::Completed(_) = outcome {
        return None;
    }
    match cut? {
        HostCut::Inactivity { silent, limit } => {
            Some(anyhow!("{}", inactivity_error_text(silent, limit)))
        }
        HostCut::NeverAdmitted { limit } => {
            Some(anyhow!("subagent {}", DispatchCut::NeverAdmitted(limit)))
        }
        HostCut::WallClock => None,
    }
}

#[cfg(test)]
#[path = "host_cuts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "host_cuts_session_tests.rs"]
mod session_tests;
