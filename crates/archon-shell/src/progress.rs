//! A renewable no-progress window. Output/activity renews it; elapsed totals do not.
use std::sync::{Arc, Mutex, OnceLock};

/// Set to `1` by a supervisor on a child whose stderr it reads as activity
/// (the host-command supervisor, the native-observation guardian launcher).
/// Only such a child writes activity lines to stderr; any other process (the
/// main run process, a terminal) records them in its log instead, where they
/// are evidence, not noise.
pub const SUPERVISED_ENV: &str = "ARCHON_ACTIVITY_SUPERVISED";
/// The stderr line a supervised child writes for observed activity.
pub const ACTIVITY_LINE: &str = "archon-host-activity: child making progress";

/// Whether this process's stderr is read as activity by its supervisor.
pub fn supervised() -> bool {
    static SUPERVISED: OnceLock<bool> = OnceLock::new();
    *SUPERVISED.get_or_init(|| std::env::var_os(SUPERVISED_ENV).is_some_and(|value| value == "1"))
}

/// Report one coalesced unit of real activity: to the supervisor when there
/// is one, else to the log. Never a saved verdict, never progress credit.
pub fn report_activity() {
    if supervised() {
        eprintln!("{ACTIVITY_LINE}");
    } else {
        tracing::debug!(target: "archon_host_activity", "child making progress");
    }
}
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone)]
pub struct Progress {
    sender: tokio::sync::watch::Sender<Instant>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    report: bool,
    last_report: Arc<Mutex<Option<Instant>>>,
    report_interval: Arc<Mutex<Duration>>,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Progress {
    pub fn new(report: bool) -> Self {
        Self::with_clock(report, Arc::new(Instant::now))
    }

    pub fn with_clock(report: bool, clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        let now = clock();
        let (sender, _) = tokio::sync::watch::channel(now);
        Self {
            sender,
            clock,
            report,
            last_report: Arc::new(Mutex::new(None)),
            report_interval: Arc::new(Mutex::new(Duration::from_secs(60))),
        }
    }

    /// Coalesce observed activity at a cadence below the caller's idle window.
    /// This changes reporting only; it never manufactures local progress.
    pub fn set_report_interval(&self, interval: Duration) {
        *self
            .report_interval
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = interval;
    }

    /// Real output or process activity only. Coalesced: no unbounded event queue.
    pub fn record(&self) {
        let now = (self.clock)();
        self.sender.send_modify(|last| *last = (*last).max(now));
        if self.report {
            let mut last = self.last_report.lock().unwrap_or_else(|e| e.into_inner());
            let interval = *self
                .report_interval
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.is_none_or(|last| now.saturating_duration_since(last) >= interval) {
                // Activity is not a saved verdict and never gets progress credit.
                report_activity();
                *last = Some(now);
            }
        }
    }

    pub fn stalled(&self, window: Duration) -> bool {
        (self.clock)().saturating_duration_since(*self.sender.borrow()) >= window
    }

    /// Drop the in-flight future at a real stall. Completion wins the boundary race.
    pub async fn bound<F: std::future::Future>(
        &self,
        window: Duration,
        future: F,
    ) -> Result<F::Output, Stalled> {
        let mut receiver = self.sender.subscribe();
        tokio::pin!(future);
        loop {
            let deadline = receiver.borrow_and_update().checked_add(window);
            tokio::select! {
                biased;
                result = &mut future => return Ok(result),
                changed = receiver.changed() => { if changed.is_err() { return Err(Stalled); } },
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => return Err(Stalled),
            }
        }
    }
}

#[derive(Debug)]
pub struct Stalled;
impl std::fmt::Display for Stalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no-progress window expired")
    }
}
impl std::error::Error for Stalled {}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
