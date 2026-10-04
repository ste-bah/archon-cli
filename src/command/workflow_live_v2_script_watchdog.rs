//! The workflow.js CPU watchdog. Host-call time is not JavaScript time, so
//! the clock pauses across each awaited host call. Issue-285: after the host's
//! terminal stop, one budget runs without resets, so a script that swallows
//! refused calls, or waits forever, cannot keep the run from ending.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[cfg(not(test))]
pub(super) const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(super) const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(super) struct WorkflowJsWatchdog {
    active_since: Arc<StdMutex<Option<Instant>>>,
    /// Issue-285: when the host's terminal stop was first seen. Never reset.
    terminal_since: Arc<StdMutex<Option<Instant>>>,
}

impl WorkflowJsWatchdog {
    pub(super) fn new() -> Self {
        Self {
            active_since: Arc::new(StdMutex::new(Some(Instant::now()))),
            terminal_since: Arc::default(),
        }
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
        if let Ok(mut active_since) = self.active_since.lock() {
            *active_since = None;
        }
    }

    pub(super) fn resume(&self) {
        if let Ok(mut active_since) = self.active_since.lock() {
            *active_since = Some(Instant::now());
        }
    }

    pub(super) fn should_interrupt(&self) -> bool {
        if self.terminal_budget_elapsed() {
            return true;
        }
        let Ok(active_since) = self.active_since.lock() else {
            return true;
        };
        active_since.is_some_and(|started| started.elapsed() >= WORKFLOW_JS_WATCHDOG)
    }
}
