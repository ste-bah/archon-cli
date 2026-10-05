//! The clock a workflow.js watchdog measures with (Issue 332).
//!
//! A QuickJS interrupt handler bounds a script that runs JavaScript without
//! end. Wall-clock time is the wrong measure for that bound: on a busy machine
//! the script's thread waits for a core, and that wait counted as script time,
//! so a script that did nothing wrong was interrupted before its first host
//! call. The CPU time of the thread that runs the JavaScript only grows while
//! that thread runs, so machine load cannot spend it.
//!
//! Every reading must come from the thread that runs the script: the runtime,
//! its interrupt handler and the host futures it awaits all live on that one
//! thread. A thread whose CPU clock cannot be read falls back to wall-clock
//! time, so the bound is never switched off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// CPU time the calling thread has used so far; `None` when it cannot be read.
#[cfg(unix)]
fn read_thread_cpu_time() -> Option<Duration> {
    #[cfg(test)]
    if tests::CLOCK_FAILS.with(std::cell::Cell::get) {
        return None;
    }
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid, writable timespec for the call.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    (status == 0).then(|| Duration::new(now.tv_sec as u64, now.tv_nsec as u32))
}

/// Platforms without a per-thread CPU clock in `libc` use the wall clock.
#[cfg(not(unix))]
fn read_thread_cpu_time() -> Option<Duration> {
    None
}

/// Warns once per process that a budget fell back to the wall clock.
fn warn_wall_clock_fallback() {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "workflow.js watchdog: the script thread's CPU clock cannot be read; \
             its budget falls back to wall-clock time"
        );
    }
}

/// A budget of JavaScript CPU time, counted from `start`.
#[derive(Clone, Copy, Debug)]
pub struct JsCpuBudget {
    /// The thread's CPU time at `start`; `None` when it could not be read.
    started_cpu: Option<Duration>,
    /// The wall clock at `start`, the fallback when the CPU clock fails.
    started_wall: Instant,
    budget: Duration,
}

impl JsCpuBudget {
    /// Starts `budget` of CPU time on the calling thread now.
    pub fn start(budget: Duration) -> Self {
        Self {
            started_cpu: read_thread_cpu_time(),
            started_wall: Instant::now(),
            budget,
        }
    }

    /// Whether the calling thread has used the whole budget since `start`.
    /// Measured in wall-clock time when the CPU clock cannot be read.
    pub fn spent(&self) -> bool {
        let used = match (self.started_cpu, read_thread_cpu_time()) {
            (Some(started), Some(now)) => now.saturating_sub(started),
            _ => {
                warn_wall_clock_fallback();
                self.started_wall.elapsed()
            }
        };
        used >= self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Test seam: this thread's CPU clock reads as failed while set.
        pub(super) static CLOCK_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    #[test]
    fn a_sleeping_thread_spends_no_budget() {
        let budget = JsCpuBudget::start(Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!budget.spent(), "time off the CPU is not script time");
    }

    #[test]
    fn a_busy_thread_spends_the_budget() {
        let budget = JsCpuBudget::start(Duration::from_millis(20));
        let mut x: u64 = 0;
        while !budget.spent() {
            x = std::hint::black_box(x.wrapping_add(1));
        }
        assert!(x > 0);
    }

    #[test]
    fn a_failed_cpu_clock_falls_back_to_the_wall_clock_and_still_bounds() {
        CLOCK_FAILS.with(|fails| fails.set(true));
        let budget = JsCpuBudget::start(Duration::from_millis(50));
        assert!(!budget.spent(), "the fallback budget starts unspent");
        std::thread::sleep(Duration::from_millis(80));
        assert!(budget.spent(), "the wall clock still ends the budget");
        CLOCK_FAILS.with(|fails| fails.set(false));
    }

    #[test]
    fn a_clock_that_fails_after_start_falls_back_to_the_wall_clock() {
        let budget = JsCpuBudget::start(Duration::from_millis(50));
        CLOCK_FAILS.with(|fails| fails.set(true));
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            budget.spent(),
            "a later failed reading never disables the bound"
        );
        CLOCK_FAILS.with(|fails| fails.set(false));
    }
}
