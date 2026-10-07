//! Host-owned dispatch clocks that count execution, never the wait for a
//! subagent slot (Issue 288).
//!
//! A dispatched call is bounded twice: the session wall clock its pipeline
//! client races against the run, and, for some calls, an outer deadline over
//! the call and its transient retries. Both used to start when the call was
//! dispatched. The executor admits a call only when a subagent slot is free,
//! so a call queued behind full slots spent its run time waiting and could be
//! cut while it had never run.
//!
//! A [`DispatchClock`] is created *pending*. It starts when the executor
//! reports that the session took its slot ([`admitted`]), whether the slot was
//! free at once or after a wait, so session and outer clocks start at the
//! same instant. A later slot wait (a retry queued again) stops it until the
//! slot is taken ([`slot_wait`]). The wait itself is bounded by no clock: each
//! slot holder is bounded by its own clocks and its inactivity bound.
//!
//! A pending clock never leaves a call unbounded. Time before admission that
//! is not a reported slot wait is counted apart, and a call whose executor
//! reports neither a slot nor a wait for a whole limit is cut with its own
//! diagnosis ([`DispatchCut::NeverAdmitted`]): a missing admission report is
//! named, never silently timed from dispatch.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy)]
struct State {
    /// Elapsed time in the current no-progress window before its latest start.
    spent: Duration,
    /// When the current no-progress window began; `None` while pending or in a slot wait.
    running_since: Option<Instant>,
    /// Whether the executor reported that the session took its slot.
    admitted: bool,
    /// Slot waits in progress. The clock runs only at zero.
    waits: usize,
    /// Pending time outside any reported slot wait.
    unreported: Duration,
    /// When the current unreported stretch began.
    unreported_since: Option<Instant>,
}

fn since(base: Duration, start: Option<Instant>, now: Instant) -> Duration {
    base + start.map_or(Duration::ZERO, |start| now.saturating_duration_since(start))
}

impl State {
    fn elapsed(&self, now: Instant) -> Duration {
        since(self.spent, self.running_since, now)
    }

    fn unreported_elapsed(&self, now: Instant) -> Duration {
        since(self.unreported, self.unreported_since, now)
    }

    /// Stop both measures, folding what ran into their totals.
    fn stop(&mut self, now: Instant) {
        self.spent = self.elapsed(now);
        self.running_since = None;
        self.unreported = self.unreported_elapsed(now);
        self.unreported_since = None;
    }

    /// Start whichever measure the state is in, outside any wait.
    fn resume(&mut self, now: Instant) {
        if self.waits > 0 {
            return;
        }
        if self.admitted {
            self.running_since = Some(now);
        } else {
            self.unreported_since = Some(now);
        }
    }
}

/// How a dispatch clock ended a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchCut {
    /// The call made no progress for the whole limit after it took its slot.
    Execution(Duration),
    /// The executor reported neither a slot nor a wait for one for the whole
    /// limit: its admission report is missing.
    NeverAdmitted(Duration),
}

impl std::fmt::Display for DispatchCut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Execution(limit) => write!(
                f,
                "made no progress for {}s after taking its subagent slot",
                limit.as_secs()
            ),
            Self::NeverAdmitted(limit) => write!(
                f,
                "never admitted: its executor reported neither a subagent slot nor a wait for one within {}s of dispatch, so its dispatch clocks never started (missing admission report)",
                limit.as_secs()
            ),
        }
    }
}

/// The execution time of one dispatched call.
#[derive(Debug)]
pub struct DispatchClock {
    state: watch::Sender<State>,
}

impl DispatchClock {
    /// A pending clock: it starts when the call is admitted to a slot.
    pub fn new() -> Arc<Self> {
        let (state, _) = watch::channel(State {
            spent: Duration::ZERO,
            running_since: None,
            admitted: false,
            waits: 0,
            unreported: Duration::ZERO,
            unreported_since: Some(Instant::now()),
        });
        Arc::new(Self { state })
    }

    /// Time in the current no-progress window.
    pub fn elapsed(&self) -> Duration {
        self.state.borrow().elapsed(Instant::now())
    }

    /// Whether the call has taken its slot.
    pub fn is_admitted(&self) -> bool {
        self.state.borrow().admitted
    }

    /// Whether a slot wait is stopping the clock now.
    pub fn waiting_for_slot(&self) -> bool {
        self.state.borrow().waits > 0
    }

    /// The call took its slot: start the clock (once; a later admission of a
    /// retry changes nothing).
    pub fn admit(&self) {
        self.state.send_modify(|state| {
            if state.admitted {
                return;
            }
            let now = Instant::now();
            state.stop(now);
            state.admitted = true;
            state.resume(now);
        });
    }

    /// A host-observed progress event renews the no-progress window.
    pub fn progress(&self) {
        self.state.send_modify(|state| {
            if state.admitted {
                state.spent = Duration::ZERO;
                state.running_since = (state.waits == 0).then(Instant::now);
            }
        });
    }

    /// Stop the clock until the returned guard drops.
    pub fn pause_for_slot(self: &Arc<Self>) -> ClockPause {
        self.state.send_modify(|state| {
            state.stop(Instant::now());
            state.waits += 1;
        });
        ClockPause(Arc::clone(self))
    }

    /// Resolves once the call has made no progress for `limit`. Never resolves
    /// before admission or while a slot wait stops the clock.
    pub async fn exceeding(&self, limit: Duration) {
        self.until(limit, State::elapsed, |state| state.running_since.is_some())
            .await
    }

    /// Resolves once `limit` of pending time passed outside any reported
    /// slot wait: the executor never reported admission.
    pub async fn unadmitted_exceeding(&self, limit: Duration) {
        self.until(limit, State::unreported_elapsed, |state| {
            state.unreported_since.is_some()
        })
        .await
    }

    /// Either bound, whichever fires first.
    pub async fn cut(&self, limit: Duration) -> DispatchCut {
        tokio::select! {
            () = self.exceeding(limit) => DispatchCut::Execution(limit),
            () = self.unadmitted_exceeding(limit) => DispatchCut::NeverAdmitted(limit),
        }
    }

    async fn until(
        &self,
        limit: Duration,
        measure: fn(&State, Instant) -> Duration,
        running: fn(&State) -> bool,
    ) {
        let mut changes = self.state.subscribe();
        loop {
            let state = *changes.borrow_and_update();
            let elapsed = measure(&state, Instant::now());
            if elapsed >= limit {
                return;
            }
            if running(&state) {
                tokio::select! {
                    () = tokio::time::sleep(limit - elapsed) => {}
                    _ = changes.changed() => {}
                }
            } else {
                // The sender lives as long as `self`, so this cannot end.
                let _ = changes.changed().await;
            }
        }
    }
}

/// Keeps one clock stopped for a slot wait; see [`DispatchClock::pause_for_slot`].
#[derive(Debug)]
pub struct ClockPause(Arc<DispatchClock>);

impl Drop for ClockPause {
    fn drop(&mut self) {
        self.0.state.send_modify(|state| {
            state.waits = state.waits.saturating_sub(1);
            state.resume(Instant::now());
        });
    }
}

/// The clocks bound to the one session its host installed them for.
///
/// Keyed by agent id, like `subagent_activity`, so a subagent the session
/// spawns (through a tool) has an id of its own and never stops the
/// parent's clocks while it waits for a slot.
#[derive(Debug, Clone)]
pub struct SessionClocks {
    pub agent_id: String,
    pub clocks: Vec<Arc<DispatchClock>>,
}

tokio::task_local! {
    static SESSION: SessionClocks;
    static CALL: Arc<DispatchClock>;
}

/// Run session `agent_id` with `clocks` stopped while it waits for a slot.
pub async fn scope_session<T>(
    agent_id: impl Into<String>,
    clocks: Vec<Arc<DispatchClock>>,
    work: impl std::future::Future<Output = T>,
) -> T {
    let session = SessionClocks {
        agent_id: agent_id.into(),
        clocks,
    };
    SESSION.scope(session, work).await
}

/// The installed session clocks, only when installed for `agent_id`.
pub fn current_for(agent_id: &str) -> Option<SessionClocks> {
    SESSION
        .try_with(Clone::clone)
        .ok()
        .filter(|session| session.agent_id == agent_id)
}

/// Task locals do not cross spawn. Capture on the caller with
/// [`current_for`] and restore around the spawned executor's work.
pub async fn inherit<T>(
    session: Option<SessionClocks>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match session {
        Some(session) => SESSION.scope(session, work).await,
        None => work.await,
    }
}

/// Run one dispatched call with `clock` as its outer deadline clock. Every
/// session the call starts on this task adds it to its own clocks.
pub async fn scope_call<T>(
    clock: Arc<DispatchClock>,
    work: impl std::future::Future<Output = T>,
) -> T {
    CALL.scope(clock, work).await
}

/// The outer deadline clock of the call running on this task, if any.
pub fn current_call() -> Option<Arc<DispatchClock>> {
    CALL.try_with(Arc::clone).ok()
}

/// Run `work` under an outer no-progress deadline of `limit`, counted
/// from the first admission of a session it starts; the cut when the
/// deadline passed first. Slot waits inside `work` do not count.
pub async fn within<T>(
    limit: Duration,
    work: impl std::future::Future<Output = T>,
) -> Result<T, DispatchCut> {
    let clock = DispatchClock::new();
    let work = scope_call(Arc::clone(&clock), work);
    tokio::select! {
        biased;
        output = work => Ok(output),
        cut = clock.cut(limit) => Err(cut),
    }
}

/// Session `agent_id` took its slot: start every clock installed for it.
/// Reported after the permit is held, whether it was free at once or came
/// after a wait. `false` when no clock is installed for it.
pub fn admitted(agent_id: &str) -> bool {
    current_for(agent_id).is_some_and(|session| {
        session.clocks.iter().for_each(|clock| clock.admit());
        true
    })
}

/// Renew the clocks for the current admitted session after a progress event.
pub fn progress() {
    if let Ok(session) = SESSION.try_with(Clone::clone) {
        session.clocks.iter().for_each(|clock| clock.progress());
    }
}

/// Session `agent_id` waits for a slot: stop every clock installed for it
/// until the returned guards drop. `None` when no clock is installed.
pub fn slot_wait(agent_id: &str) -> Option<Vec<ClockPause>> {
    current_for(agent_id).map(|session| {
        session
            .clocks
            .iter()
            .map(DispatchClock::pause_for_slot)
            .collect()
    })
}

#[cfg(test)]
#[path = "subagent_dispatch_clock_tests.rs"]
mod tests;
