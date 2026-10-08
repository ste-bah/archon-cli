//! Host-command work that must finish before the process exits (#297 r9).
//!
//! A cancelled call's tree is torn down, and its staging sealed, on a thread
//! of its own, after the call's future is gone. Each such piece of work
//! holds a [`PendingWork`] from before it starts until it ends, on every
//! platform. At process exit the pending set is drained: the exit waits
//! while the work makes progress, and gives up after [`NO_PROGRESS`] without
//! any (or [`CAP`] in all), so a hung teardown cannot hold the exit open.
//! Work the drain gives up on, or that a hard kill ends, leaves the call's
//! residue record (`workflow_host_staging_residue`): the next resume clears
//! that staging before any child runs.
//!
//! The drain runs from a C exit hook (`atexit`). That covers a return from
//! `main` on every platform and `std::process::exit` on Unix. On Windows
//! `std::process::exit` ends the process without the hook, so the CLI's own
//! exits call [`drain_before_exit`] first.
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long the exit waits for the pending work to make any progress.
pub(crate) const NO_PROGRESS: Duration = Duration::from_secs(10);
/// How long the exit waits in all.
pub(crate) const CAP: Duration = Duration::from_secs(60);

struct State {
    active: usize,
    /// Bumped by every start, end and progress report.
    progress: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    active: 0,
    progress: 0,
});
static CHANGED: Condvar = Condvar::new();
static EXIT_HOOK: OnceLock<i32> = OnceLock::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|error| error.into_inner())
}

fn changed(update: impl FnOnce(&mut State)) {
    let mut state = state();
    update(&mut state);
    state.progress = state.progress.wrapping_add(1);
    drop(state);
    CHANGED.notify_all();
}

/// One piece of teardown or sealing work in flight. The exit waits for it
/// until it is dropped.
pub(crate) struct PendingWork {
    _private: (),
}

impl PendingWork {
    pub(crate) fn begin() -> Self {
        install_exit_hook();
        changed(|state| state.active += 1);
        Self { _private: () }
    }

    /// The work finished a phase: the exit keeps waiting for it.
    pub(crate) fn progressed(&self) {
        changed(|_| {});
    }
}

impl Drop for PendingWork {
    fn drop(&mut self) {
        changed(|state| state.active = state.active.saturating_sub(1));
    }
}

/// Waits until no work is pending. False: the work made no progress for
/// `no_progress`, or did not end within `cap`; it is still unknown.
pub(crate) fn drain(no_progress: Duration, cap: Duration) -> bool {
    let begun = Instant::now();
    let cap_at = begun + cap;
    let mut state = state();
    let mut seen = state.progress;
    let mut stall_at = begun + no_progress;
    loop {
        if state.active == 0 {
            return true;
        }
        let now = Instant::now();
        if state.progress != seen {
            seen = state.progress;
            stall_at = now + no_progress;
        }
        let until = stall_at.min(cap_at);
        if now >= until {
            return false;
        }
        state = match CHANGED.wait_timeout(state, until - now) {
            Ok((state, _)) => state,
            Err(error) => error.into_inner().0,
        };
    }
}

/// The bounded drain the process runs before it exits.
pub(crate) fn drain_before_exit() {
    if !drain(NO_PROGRESS, CAP) {
        tracing::warn!(
            "host command teardown or sealing did not finish before exit; its residue record stays for the next resume"
        );
    }
}

extern "C" fn exit_drain() {
    let _ = std::panic::catch_unwind(drain_before_exit);
}

fn install_exit_hook() {
    let result = EXIT_HOOK.get_or_init(|| {
        // SAFETY: the callback has C ABI and static lifetime, and it
        // catches panics, so none unwinds into the C runtime.
        unsafe { libc::atexit(exit_drain) }
    });
    if *result != 0 {
        tracing::warn!("host command exit drain unavailable; residue records cover the exit");
    }
}

#[cfg(test)]
#[path = "workflow_host_exit_drain_tests.rs"]
mod tests;
