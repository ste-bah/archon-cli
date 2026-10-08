//! The stream idle window, measured on more than one clock (Issue 364).
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
//! passed; the monotonic clock still bounds the window.
//!
//! A wall clock stepped FORWARD (NTP at a wake, an operator) is not a sleep,
//! and it must not cut a healthy stream. So the wall clock ends the window only
//! when a clock that cannot be set agrees: the boot clock, which counts time
//! asleep. That is `CLOCK_MONOTONIC_RAW` on Apple platforms (the
//! `mach_continuous_time` clock; Apple's `CLOCK_MONOTONIC` is the wall clock
//! minus the boot time, so it is not independent of the wall clock),
//! `CLOCK_BOOTTIME` on Linux, and `GetTickCount64` on Windows (the
//! interrupt-time count that includes sleep; `QueryUnbiasedInterruptTime`
//! excludes it). A step moves the wall clock alone; a sleep moves both. Where
//! no boot clock can be read, the wall clock ends the window only after
//! [`STEP_FLOOR`] (or the whole limit, when it is shorter) of monotonic
//! silence too.
//!
//! It is a no-progress window like before: every event the caller receives
//! starts a new one. A window that ends because of a sleep says so
//! ([`IdleExpired::slept`]), so the caller can measure its own no-progress
//! budget from the wake and not from before the sleep.

use std::future::Future;
use std::time::{Duration, SystemTime};

/// How often a waiting window re-reads the wall clock.
#[cfg(not(test))]
const WALL_RECHECK: Duration = Duration::from_secs(5);
#[cfg(test)]
const WALL_RECHECK: Duration = Duration::from_millis(20);

/// Without a boot clock: the monotonic silence a wall-clock end also needs.
#[cfg(not(test))]
const STEP_FLOOR: Duration = Duration::from_secs(60);
#[cfg(test)]
const STEP_FLOOR: Duration = Duration::from_millis(150);

/// The window ended before the awaited work did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IdleExpired {
    /// The wall and boot clocks ended it: the machine slept through it.
    pub(super) slept: bool,
}

#[cfg(test)]
thread_local! {
    /// Test seam: how far this thread's wall clock has jumped forward, as a
    /// wake from a long sleep or a clock step jumps it.
    pub(super) static WALL_JUMP: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    /// Test seam: how far this thread's boot clock has jumped forward, as a
    /// sleep (and never a clock step) jumps it.
    pub(super) static BOOT_JUMP: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    /// Test seam: this thread reads no boot clock.
    pub(super) static NO_BOOT_CLOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn wall_now() -> SystemTime {
    let now = SystemTime::now();
    #[cfg(test)]
    let now = now + WALL_JUMP.with(std::cell::Cell::get);
    now
}

/// The boot clock: monotonic, never set, and counting time asleep.
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
fn read_boot_clock() -> Option<Duration> {
    #[cfg(target_vendor = "apple")]
    let clock = libc::CLOCK_MONOTONIC_RAW;
    #[cfg(not(target_vendor = "apple"))]
    let clock = libc::CLOCK_BOOTTIME;
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid, writable timespec for the call.
    let status = unsafe { libc::clock_gettime(clock, &mut now) };
    (status == 0).then(|| Duration::new(now.tv_sec as u64, now.tv_nsec as u32))
}

/// The boot clock on Windows: milliseconds since boot, sleep included.
#[cfg(windows)]
fn read_boot_clock() -> Option<Duration> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTickCount64() -> u64;
    }
    // SAFETY: `GetTickCount64` takes no arguments and cannot fail.
    Some(Duration::from_millis(unsafe { GetTickCount64() }))
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn read_boot_clock() -> Option<Duration> {
    None
}

fn boot_now() -> Option<Duration> {
    #[cfg(test)]
    if NO_BOOT_CLOCK.with(std::cell::Cell::get) {
        return None;
    }
    let now = read_boot_clock();
    #[cfg(test)]
    let now = now.map(|now| now + BOOT_JUMP.with(std::cell::Cell::get));
    now
}

/// Run `work` until it finishes, or until `limit` has passed on the monotonic
/// clock, or on the wall clock confirmed as a sleep (see the module doc).
/// `work` must be cancel safe: it is polled across re-checks and dropped when
/// the window ends.
pub(super) async fn within<F: Future>(limit: Duration, work: F) -> Result<F::Output, IdleExpired> {
    let started = tokio::time::Instant::now();
    let wall_started = wall_now();
    let boot_started = boot_now();
    let mut work = std::pin::pin!(work);
    loop {
        let left = limit.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Err(IdleExpired { slept: false });
        }
        if slept_through(limit, started, wall_started, boot_started) {
            return Err(IdleExpired { slept: true });
        }
        if let Ok(output) = tokio::time::timeout(left.min(WALL_RECHECK), &mut work).await {
            return Ok(output);
        }
    }
}

/// Has the wall clock passed the whole `limit`, and is that a sleep rather
/// than a step of the wall clock?
fn slept_through(
    limit: Duration,
    started: tokio::time::Instant,
    wall_started: SystemTime,
    boot_started: Option<Duration>,
) -> bool {
    let wall_silent = wall_now()
        .duration_since(wall_started)
        .unwrap_or(Duration::ZERO);
    if wall_silent < limit {
        return false;
    }
    match (boot_started, boot_now()) {
        (Some(from), Some(now)) => now.saturating_sub(from) >= limit,
        _ => started.elapsed() >= limit.min(STEP_FLOOR),
    }
}

#[cfg(test)]
#[path = "stream_idle_window_tests.rs"]
mod tests;
