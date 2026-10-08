//! One inactivity clock shared by blocking teardown and its async watchdog.
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Progress {
    bound: Duration,
    last: Arc<Mutex<Instant>>,
}
impl Progress {
    pub fn new(bound: Duration) -> Self {
        Self {
            bound,
            last: Arc::new(Mutex::new(Instant::now())),
        }
    }
    /// Call only for measurable work: an identity read or decreased membership.
    pub fn advance(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }
    pub fn remaining(&self) -> Duration {
        self.bound.saturating_sub(
            self.last
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed(),
        )
    }
    pub fn check(&self) -> io::Result<()> {
        if self.remaining().is_zero() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "teardown made no progress within its inactivity bound",
            ))
        } else {
            Ok(())
        }
    }
    /// Includes blocking-pool admission, without limiting progressing work.
    pub async fn watch<T>(&self, mut task: tokio::task::JoinHandle<T>) -> io::Result<T> {
        loop {
            tokio::select! {
                result = &mut task => return result.map_err(io::Error::other),
                _ = tokio::time::sleep(self.remaining()) => self.check()?,
            }
        }
    }
}

/// Poll accounting until empty or membership stops decreasing for the bound.
pub fn confirm_empty(
    progress: &Progress,
    mut probe: impl FnMut() -> io::Result<u32>,
) -> io::Result<u32> {
    let mut previous = None;
    loop {
        let active = probe()?;
        if active == 0 {
            return Ok(0);
        }
        if previous.is_none_or(|last| active < last) {
            progress.advance();
        }
        previous = Some(active);
        if progress.check().is_err() {
            return Ok(active);
        }
        std::thread::sleep(Duration::from_millis(10).min(progress.remaining()));
    }
}

#[cfg(test)]
#[path = "teardown_progress_tests.rs"]
mod tests;
