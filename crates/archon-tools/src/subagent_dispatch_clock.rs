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
//! A [`DispatchClock`] runs while its call executes and stops while the call
//! waits for a slot. The executor reports the wait where it happens, with
//! [`slot_wait`]: the guard it returns stops every clock installed for that
//! session until the slot is acquired. The wait itself is bounded by no
//! clock. Each slot holder is bounded by its own clocks and its inactivity
//! bound, so a slot always comes free unless every holder makes progress.
//!
//! The clock runs from the moment it is created. An executor that has no
//! slots never reports a wait, and its calls are bounded exactly as before:
//! a missing report can never leave a call unbounded.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy)]
struct State {
    /// Execution time counted before the current run began.
    spent: Duration,
    /// When the current run began; `None` while a slot wait holds it.
    running_since: Option<Instant>,
    /// Slot waits in progress. The clock runs only at zero.
    waits: usize,
}

impl State {
    fn elapsed(&self, now: Instant) -> Duration {
        self.spent
            + self
                .running_since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }
}

/// The execution time of one dispatched call.
#[derive(Debug)]
pub struct DispatchClock {
    state: watch::Sender<State>,
}

impl DispatchClock {
    /// A clock that runs from now until a slot wait stops it.
    pub fn new() -> Arc<Self> {
        let (state, _) = watch::channel(State {
            spent: Duration::ZERO,
            running_since: Some(Instant::now()),
            waits: 0,
        });
        Arc::new(Self { state })
    }

    /// Execution time counted so far.
    pub fn elapsed(&self) -> Duration {
        self.state.borrow().elapsed(Instant::now())
    }

    /// Whether a slot wait is stopping the clock now.
    pub fn waiting_for_slot(&self) -> bool {
        self.state.borrow().waits > 0
    }

    /// Stop the clock until the returned guard drops.
    pub fn pause_for_slot(self: &Arc<Self>) -> ClockPause {
        self.state.send_modify(|state| {
            if state.waits == 0 {
                let now = Instant::now();
                state.spent = state.elapsed(now);
                state.running_since = None;
            }
            state.waits += 1;
        });
        ClockPause(Arc::clone(self))
    }

    /// Resolves once the call has executed for `limit`. Never resolves
    /// while a slot wait stops the clock.
    pub async fn exceeding(&self, limit: Duration) {
        let mut changes = self.state.subscribe();
        loop {
            let state = *changes.borrow_and_update();
            match state.running_since {
                Some(_) => {
                    let elapsed = state.elapsed(Instant::now());
                    if elapsed >= limit {
                        return;
                    }
                    tokio::select! {
                        () = tokio::time::sleep(limit - elapsed) => {}
                        _ = changes.changed() => {}
                    }
                }
                None => {
                    // The sender lives as long as `self`, so this cannot end.
                    let _ = changes.changed().await;
                }
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
            if state.waits == 0 {
                state.running_since = Some(Instant::now());
            }
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

/// Run `work` under an outer deadline of `limit` execution time; `None`
/// when the deadline passed first. Slot waits inside `work` do not count.
pub async fn within<T>(limit: Duration, work: impl std::future::Future<Output = T>) -> Option<T> {
    let clock = DispatchClock::new();
    let work = scope_call(Arc::clone(&clock), work);
    tokio::select! {
        biased;
        output = work => Some(output),
        () = clock.exceeding(limit) => None,
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
