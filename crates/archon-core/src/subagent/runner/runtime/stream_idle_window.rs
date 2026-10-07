//! The stream idle window, measured on both clocks (Issue 364).
//!
//! tokio's clock is monotonic, and on macOS a monotonic clock stops while the
//! machine sleeps. A window measured on it alone resumes after a wake where it
//! stopped, as if the sleep had not happened: a stream opened just before a
//! seven-hour sleep still has most of its window left after the wake, although
//! its provider gave up on the request hours ago and its socket is dead.
//!
//! So the window also ends when the wall clock says the stream has been silent
//! for the whole limit. The wall clock is re-read at least every
//! [`WALL_RECHECK`], so after a wake the window ends within that period and the
//! round resends its request. A wall clock set backwards only reads as no time
//! passed; the monotonic clock still bounds the window. It is a no-progress
//! window like before: every event the caller receives starts a new one.

use std::future::Future;
use std::time::{Duration, SystemTime};

/// How often a waiting window re-reads the wall clock.
#[cfg(not(test))]
const WALL_RECHECK: Duration = Duration::from_secs(5);
#[cfg(test)]
const WALL_RECHECK: Duration = Duration::from_millis(20);

/// The window ended before the awaited work did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IdleExpired;

#[cfg(test)]
thread_local! {
    /// Test seam: how far this thread's wall clock has jumped forward, as a
    /// wake from a long sleep jumps it while the monotonic clock does not.
    pub(super) static WALL_JUMP: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

fn wall_now() -> SystemTime {
    let now = SystemTime::now();
    #[cfg(test)]
    let now = now + WALL_JUMP.with(std::cell::Cell::get);
    now
}

/// Run `work` until it finishes, or until `limit` has passed on either the
/// monotonic or the wall clock. `work` must be cancel safe: it is polled
/// across re-checks and dropped when the window ends.
pub(super) async fn within<F: Future>(limit: Duration, work: F) -> Result<F::Output, IdleExpired> {
    let started = tokio::time::Instant::now();
    let wall_started = wall_now();
    let mut work = std::pin::pin!(work);
    loop {
        let wall_silent = wall_now()
            .duration_since(wall_started)
            .unwrap_or(Duration::ZERO);
        let left = limit.saturating_sub(started.elapsed());
        if left.is_zero() || wall_silent >= limit {
            return Err(IdleExpired);
        }
        if let Ok(output) = tokio::time::timeout(left.min(WALL_RECHECK), &mut work).await {
            return Ok(output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn work_that_finishes_inside_the_window_is_returned() {
        let out = within(Duration::from_secs(5), async { 7 }).await;
        assert_eq!(out, Ok(7));
    }

    #[tokio::test]
    async fn the_monotonic_clock_still_ends_a_silent_window() {
        let started = std::time::Instant::now();
        let out = within(Duration::from_millis(60), std::future::pending::<()>()).await;
        assert_eq!(out, Err(IdleExpired));
        assert!(started.elapsed() >= Duration::from_millis(60));
    }

    #[tokio::test]
    async fn a_wall_clock_jump_ends_the_window_soon_after_the_wake() {
        // A provider stream that went silent: open, never sending.
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<u8>(1);
        let jump = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            WALL_JUMP.with(|jump| jump.set(Duration::from_secs(7 * 3600)));
        };
        let started = std::time::Instant::now();
        let (out, ()) = tokio::join!(within(Duration::from_secs(3600), rx.recv()), jump);
        WALL_JUMP.with(|jump| jump.set(Duration::ZERO));
        assert_eq!(out, Err(IdleExpired));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the window must end within a re-check of the wake, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_wall_clock_set_backwards_does_not_end_the_window() {
        WALL_JUMP.with(|jump| jump.set(Duration::from_secs(3600)));
        let set_back = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            WALL_JUMP.with(|jump| jump.set(Duration::ZERO));
        };
        let (out, ()) = tokio::join!(
            within(Duration::from_millis(200), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                1
            }),
            set_back
        );
        assert_eq!(out, Ok(1));
    }
}
