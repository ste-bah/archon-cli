//! A scoped virtual no-progress clock for the synchronous cleanup budget test.
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
thread_local! {
    static NOW: Cell<Option<Instant>> = const { Cell::new(None) };
    static REMOVAL_WAIT: Cell<Option<Duration>> = const { Cell::new(None) };
}

pub(super) fn now() -> Option<Instant> {
    NOW.with(Cell::get)
}

pub(crate) struct ClockGuard;
impl Drop for ClockGuard {
    fn drop(&mut self) {
        NOW.with(|now| now.set(None));
        REMOVAL_WAIT.with(|wait| wait.set(None));
    }
}

/// Freeze the clock after fixture readiness, then charge the next worktree
/// removal an exact period of no progress once its child has been spawned.
pub(crate) fn delay_next_removal(elapsed: Duration) -> ClockGuard {
    NOW.with(|now| {
        assert!(now.get().is_none());
        now.set(Some(Instant::now()));
    });
    REMOVAL_WAIT.with(|wait| wait.set(Some(elapsed)));
    ClockGuard
}

/// Whether the delay armed by [`delay_next_removal`] was charged to a
/// worktree removal on this thread.
#[cfg(unix)]
pub(crate) fn removal_charged() -> bool {
    REMOVAL_WAIT.with(|wait| wait.get().is_none())
}

pub(super) fn after_spawn(args: &[&str]) {
    if args.starts_with(&["worktree", "remove"]) {
        REMOVAL_WAIT.with(|wait| {
            if let Some(elapsed) = wait.take() {
                NOW.with(|now| now.set(Some(now.get().unwrap() + elapsed)));
            }
        });
    }
}

#[test]
fn a_stall_at_the_profile_window_still_refuses() {
    use std::sync::{Arc, atomic::AtomicBool};
    let _clock = delay_next_removal(Duration::from_secs(20));
    let control = super::Control::new(20, Arc::new(AtomicBool::new(false)));
    after_spawn(&["worktree", "remove"]);
    assert!(
        control
            .poll("gated removal")
            .unwrap_err()
            .to_string()
            .contains(super::OBSERVATION_STALLED)
    );
}

#[test]
fn a_six_second_wait_exceeds_the_old_window_and_fits_the_profile() {
    use std::sync::{Arc, atomic::AtomicBool};
    let _clock = delay_next_removal(Duration::from_secs(6));
    let old = super::Control::new(5, Arc::new(AtomicBool::new(false)));
    let profile = super::Control::new(20, Arc::new(AtomicBool::new(false)));
    after_spawn(&["worktree", "remove"]);
    assert!(profile.poll("gated removal").is_ok());
    assert!(old.poll("gated removal").is_err());
}
