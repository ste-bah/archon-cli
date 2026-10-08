//! A contention wait ends on inactivity, never on a total clock or count.
//!
//! Each retry asks the progress window whether any writer committed since the
//! last look. Progress renews the window. One full window with no progress
//! ends the wait as a typed `StoreBusy` pause that names the store, so the
//! caller can resume the operation later. A long wait stays visible at warn
//! level, once per [`LONG_WAIT_NOTICE`].
use std::time::{Duration, Instant};

use crate::CozoGuardConfig;
use crate::progress::Window;

/// How often a wait that is still going repeats its warning.
pub(crate) const LONG_WAIT_NOTICE: Duration = Duration::from_secs(5);

/// Rate limit for the warn-level account of one long wait.
pub(crate) struct Notice {
    started: Instant,
    next: Duration,
}

impl Notice {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            next: LONG_WAIT_NOTICE,
        }
    }

    /// The time waited so far, once per [`LONG_WAIT_NOTICE`]; else `None`.
    pub(crate) fn due(&mut self) -> Option<Duration> {
        let waited = self.started.elapsed();
        if waited < self.next {
            return None;
        }
        self.next = waited + LONG_WAIT_NOTICE;
        Some(waited)
    }
}

/// The retry state of one guarded operation that met contention.
pub(crate) struct Contention {
    window: Option<Window>,
    notice: Notice,
    attempts: usize,
    step: usize,
}

impl Contention {
    pub(crate) fn new() -> Self {
        Self {
            window: None,
            notice: Notice::new(),
            attempts: 0,
            step: 0,
        }
    }

    /// The sleep before the next attempt, or the typed pause once a full
    /// no-progress window has passed since the last observed progress.
    pub(crate) fn next(
        &mut self,
        context: &str,
        config: &CozoGuardConfig,
        error: &str,
    ) -> anyhow::Result<Duration> {
        let wait = config.busy_wait;
        let window = self
            .window
            .get_or_insert_with(|| Window::for_config(config, wait));
        self.attempts += 1;
        let store = describe(config);
        let Some(remaining) = window.remaining() else {
            tracing::warn!(
                context,
                attempts = self.attempts,
                %store,
                no_progress_ms = wait.as_millis() as u64,
                error,
                "Cozo store stayed busy with no writer progress; pausing the operation"
            );
            return Err(crate::StoreBusy {
                context: context.into(),
                attempts: self.attempts,
                detail: format!(
                    "{store} stayed busy with no writer progress for {}ms; last error: {error}",
                    wait.as_millis()
                ),
            }
            .into());
        };
        if let Some(waited) = self.notice.due() {
            tracing::warn!(
                context,
                attempts = self.attempts,
                %store,
                waited_ms = waited.as_millis() as u64,
                pause_after_ms = remaining.as_millis() as u64,
                error,
                "Cozo store busy; still waiting while writers make progress"
            );
        } else {
            tracing::trace!(
                context,
                attempts = self.attempts,
                error,
                "Cozo store busy; retrying guarded operation"
            );
        }
        let backoff = crate::retry::backoff_duration(config, self.step).min(remaining);
        self.step = (self.step + 1).min(crate::retry::backoff_steps(config) - 1);
        Ok(backoff)
    }
}

fn describe(config: &CozoGuardConfig) -> String {
    match &config.write_lock_path {
        Some(lock) => format!("Cozo store guarded by write lock {}", lock.display()),
        None => "Cozo store with no write lock path (only this process's writes count as progress)"
            .into(),
    }
}
