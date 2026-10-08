//! When a call's process trees are gone (#297 round 8).
//!
//! A supervisor that is dropped (the call was cancelled) tears its tree down
//! on a thread of its own, and TERM then KILL can take seconds. Until then
//! the child can still write staging, so the call seals its staging only
//! after every teardown it tracked has reported. Each supervised tree holds
//! a [`TeardownToken`] of the call's latch; the token reports when the
//! teardown is settled, or as unconfirmed if it is dropped first.
use std::sync::{Arc, Mutex, MutexGuard};

/// Runs once no tracked teardown is pending. The flag: every teardown was
/// confirmed (no process of the call can still write).
pub(crate) type AfterTeardown = Box<dyn FnOnce(bool) + Send>;

#[derive(Clone, Default)]
pub(crate) struct TeardownLatch(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    pending: usize,
    unconfirmed: bool,
    waiting: Vec<AfterTeardown>,
}

impl std::fmt::Debug for TeardownLatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        formatter
            .debug_struct("TeardownLatch")
            .field("pending", &state.pending)
            .field("unconfirmed", &state.unconfirmed)
            .finish()
    }
}

impl TeardownLatch {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// A tree is starting: the latch waits for its token to report.
    pub(crate) fn track(&self) -> TeardownToken {
        self.state().pending += 1;
        TeardownToken {
            latch: Some(self.clone()),
        }
    }

    /// Whether a tracked teardown has not reported yet.
    pub(crate) fn pending(&self) -> bool {
        self.state().pending > 0
    }

    /// Runs `then` now, on this thread, when no teardown is pending;
    /// otherwise on the thread that settles the last pending teardown.
    pub(crate) fn after_teardown(&self, then: AfterTeardown) {
        let mut state = self.state();
        if state.pending > 0 {
            state.waiting.push(then);
            return;
        }
        let confirmed = !state.unconfirmed;
        drop(state);
        then(confirmed);
    }

    fn report(&self, confirmed: bool) {
        let mut state = self.state();
        state.pending = state.pending.saturating_sub(1);
        state.unconfirmed |= !confirmed;
        if state.pending > 0 {
            return;
        }
        let waiting = std::mem::take(&mut state.waiting);
        let confirmed = !state.unconfirmed;
        drop(state);
        for then in waiting {
            then(confirmed);
        }
    }
}

/// One tracked tree. Dropped without [`TeardownToken::settled`] (the
/// teardown thread never ran, or panicked), it reports unconfirmed.
pub(crate) struct TeardownToken {
    latch: Option<TeardownLatch>,
}

impl TeardownToken {
    pub(crate) fn settled(mut self, confirmed: bool) {
        if let Some(latch) = self.latch.take() {
            latch.report(confirmed);
        }
    }
}

impl Drop for TeardownToken {
    fn drop(&mut self) {
        if let Some(latch) = self.latch.take() {
            latch.report(false);
        }
    }
}

#[cfg(test)]
#[path = "workflow_host_command_teardown_latch_tests.rs"]
mod tests;
