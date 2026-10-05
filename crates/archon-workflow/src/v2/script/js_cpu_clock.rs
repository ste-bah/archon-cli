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
//! thread.

use std::time::Duration;

/// CPU time the calling thread has used so far.
#[cfg(unix)]
pub fn js_thread_cpu_time() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid, writable timespec for the call.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    if status != 0 {
        return Duration::ZERO;
    }
    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

/// Platforms without a per-thread CPU clock in `libc` measure the time since
/// the first reading on this thread instead.
#[cfg(not(unix))]
pub fn js_thread_cpu_time() -> Duration {
    thread_local! {
        static ORIGIN: std::time::Instant = std::time::Instant::now();
    }
    ORIGIN.with(|origin| origin.elapsed())
}

/// A budget of JavaScript CPU time, counted from `start`.
#[derive(Clone, Copy, Debug)]
pub struct JsCpuBudget {
    started: Duration,
    budget: Duration,
}

impl JsCpuBudget {
    /// Starts `budget` of CPU time on the calling thread now.
    pub fn start(budget: Duration) -> Self {
        Self {
            started: js_thread_cpu_time(),
            budget,
        }
    }

    /// Whether the calling thread has used the whole budget since `start`.
    pub fn spent(&self) -> bool {
        js_thread_cpu_time().saturating_sub(self.started) >= self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
