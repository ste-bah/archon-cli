//! A renewable idle window shared by all phases and both output readers.
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use super::RunError;

#[derive(Clone, Copy)]
struct Progress {
    last_output: Instant,
    expired: bool,
}

pub(in crate::hooks) struct NoProgressWindow {
    window: Duration,
    progress: watch::Sender<Progress>,
}

impl NoProgressWindow {
    pub(in crate::hooks) fn new(window: Duration) -> Arc<Self> {
        let (progress, _) = watch::channel(Progress {
            last_output: Instant::now(),
            expired: window.is_zero(),
        });
        Arc::new(Self { window, progress })
    }

    /// Even bytes discarded by the capture limit renew the idle window.
    /// Once silence has exceeded the window, later output cannot undo it.
    pub(in crate::hooks) fn record_output(&self) {
        self.progress.send_modify(|progress| {
            let now = Instant::now();
            progress.expired |= now.duration_since(progress.last_output) >= self.window;
            progress.last_output = now;
        });
    }

    // Sampling the clock, checking current state, and latching expiry must be
    // one operation under the same lock that records output. Never decide
    // expiry from the timer's (possibly superseded) deadline.
    fn current_deadline(&self) -> Result<Instant, ()> {
        #[cfg(test)]
        before_expiry_check();
        let mut deadline = Err(());
        self.progress.send_if_modified(|progress| {
            let was_expired = progress.expired;
            progress.expired |= progress.last_output.elapsed() >= self.window;
            if !progress.expired {
                deadline = Ok(progress.last_output + self.window);
            }
            progress.expired != was_expired
        });
        deadline
    }

    pub(in crate::hooks) fn expired(&self) -> bool {
        self.current_deadline().is_err()
    }

    pub(in crate::hooks) async fn wait<T>(
        &self,
        phase: &'static str,
        future: impl Future<Output = T>,
    ) -> Result<T, RunError> {
        tokio::pin!(future);
        let mut updates = self.progress.subscribe();
        loop {
            updates.borrow_and_update();
            let deadline = self
                .current_deadline()
                .map_err(|()| RunError::Timeout(phase))?;
            tokio::select! {
                biased;
                _ = updates.changed() => {},
                _ = tokio::time::sleep_until(deadline) => {},
                value = &mut future => {
                    return if self.expired() { Err(RunError::Timeout(phase)) } else { Ok(value) };
                }
            }
        }
    }
}

// Test-only scheduling seam: suspend immediately before expiry evaluation.
#[cfg(test)]
thread_local! {
    static BEFORE_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
fn before_expiry_check() {
    if let Some(action) = BEFORE_CHECK.with(|slot| slot.borrow_mut().take()) {
        action();
    }
}

#[cfg(test)]
#[path = "executor_progress_race_tests.rs"]
mod race_tests;
