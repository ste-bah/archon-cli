//! A renewable no-progress window. Output/activity renews it; elapsed totals do not.
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone)]
pub struct Progress {
    sender: tokio::sync::watch::Sender<Instant>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    report: bool,
    last_report: Arc<Mutex<Instant>>,
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
            last_report: Arc::new(Mutex::new(now)),
        }
    }

    /// Real output or process activity only. Coalesced: no unbounded event queue.
    pub fn record(&self) {
        let now = (self.clock)();
        self.sender.send_modify(|last| *last = (*last).max(now));
        if self.report {
            let mut last = self.last_report.lock().unwrap_or_else(|e| e.into_inner());
            if now.saturating_duration_since(*last) >= Duration::from_secs(60) {
                // Activity is not a saved verdict and never gets progress credit.
                eprintln!("archon-host-activity: child making progress");
                *last = now;
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
