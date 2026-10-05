//! The workflow.js CPU watchdog. Host-call time is not JavaScript time, so
//! the budget restarts after each awaited host call. Issue-285: after the
//! host's terminal stop, one budget runs without resets, so a script that
//! swallows refused calls, or waits forever, cannot keep the run from ending.
//!
//! Issue 332: the JavaScript budget is CPU time of the script's thread, not
//! wall-clock time. On a loaded machine that thread waits for a core, and a
//! wall-clock budget spent on that wait interrupted scripts that had done
//! nothing wrong before their first host call. CPU time grows only while the
//! script itself runs, so only a script that computes without calling the
//! host for a whole budget is interrupted, however busy the machine is. The
//! QuickJS interrupt handler, `pause`, `resume` and the budget's start all run
//! on that one thread (the runner's current-thread runtime).
//!
//! The post-stop budget stays wall-clock on purpose: a script idle after the
//! stop uses no CPU, and the host's stop has already decided the outcome, so
//! that budget can only end the wait, never fail a run.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use archon_workflow::v2::script::js_cpu_clock::JsCpuBudget;

#[cfg(not(test))]
pub(super) const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(super) const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(super) struct WorkflowJsWatchdog {
    /// The CPU budget of the JavaScript running now; `None` while the script
    /// awaits a host call.
    active: Arc<StdMutex<Option<JsCpuBudget>>>,
    /// Issue-285: when the host's terminal stop was first seen. Never reset.
    terminal_since: Arc<StdMutex<Option<Instant>>>,
}

impl WorkflowJsWatchdog {
    /// Starts the budget on the calling (script) thread and installs it as
    /// `runtime`'s interrupt handler.
    pub(super) async fn install(runtime: &rquickjs::AsyncRuntime) -> Self {
        let watchdog = Self {
            active: Arc::new(StdMutex::new(Some(JsCpuBudget::start(
                WORKFLOW_JS_WATCHDOG,
            )))),
            terminal_since: Arc::default(),
        };
        let for_interrupt = watchdog.clone();
        runtime
            .set_interrupt_handler(Some(Box::new(move || for_interrupt.should_interrupt())))
            .await;
        watchdog
    }

    pub(super) fn start_terminal_budget(&self) {
        if let Ok(mut since) = self.terminal_since.lock() {
            since.get_or_insert_with(Instant::now);
        }
    }

    fn terminal_budget_elapsed(&self) -> bool {
        let Ok(since) = self.terminal_since.lock() else {
            return true;
        };
        since.is_some_and(|started| started.elapsed() >= WORKFLOW_JS_WATCHDOG)
    }

    /// Resolves once the post-stop budget is spent; pending before any stop.
    pub(super) async fn terminal_budget_spent(&self) {
        while !self.terminal_budget_elapsed() {
            tokio::time::sleep(WORKFLOW_JS_WATCHDOG / 10).await;
        }
    }

    pub(super) fn pause(&self) {
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }

    pub(super) fn resume(&self) {
        if let Ok(mut active) = self.active.lock() {
            *active = Some(JsCpuBudget::start(WORKFLOW_JS_WATCHDOG));
        }
    }

    pub(super) fn should_interrupt(&self) -> bool {
        if self.terminal_budget_elapsed() {
            return true;
        }
        let Ok(active) = self.active.lock() else {
            return true;
        };
        active.is_some_and(|budget| budget.spent())
    }
}

/// Issue 332 test seam: how long the script thread of a run stalls, off the
/// CPU, after its engine started and before its script is evaluated, as a
/// loaded machine stalls it.
#[cfg(test)]
pub(super) static ENGINE_START_STALLS: std::sync::LazyLock<
    StdMutex<std::collections::BTreeMap<String, Duration>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(test)]
pub(super) fn stall_engine_start(run_id: &str) {
    let stall = ENGINE_START_STALLS
        .lock()
        .ok()
        .and_then(|stalls| stalls.get(run_id).copied());
    if let Some(stall) = stall {
        std::thread::sleep(stall);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn time_off_the_cpu_never_spends_the_script_budget() {
        let runtime = rquickjs::AsyncRuntime::new().expect("runtime");
        let watchdog = WorkflowJsWatchdog::install(&runtime).await;
        std::thread::sleep(WORKFLOW_JS_WATCHDOG * 2);
        assert!(!watchdog.should_interrupt());
        watchdog.pause();
        watchdog.resume();
        std::thread::sleep(WORKFLOW_JS_WATCHDOG * 2);
        assert!(!watchdog.should_interrupt());
    }

    #[tokio::test]
    async fn a_script_computing_for_a_whole_budget_is_interrupted() {
        let runtime = rquickjs::AsyncRuntime::new().expect("runtime");
        let watchdog = WorkflowJsWatchdog::install(&runtime).await;
        let mut spins: u64 = 0;
        while !watchdog.should_interrupt() {
            spins = std::hint::black_box(spins.wrapping_add(1));
        }
        assert!(spins > 0);
    }
}
