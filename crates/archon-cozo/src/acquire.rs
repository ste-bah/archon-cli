//! Queued write-lock acquisition under one no-progress window.
//!
//! `fd_lock` offers only a non-blocking `try_write` and an unbounded blocking
//! `write`, and an unbounded wait turns a stuck holder into a hung process.
//! So acquisition polls, backing off from [`ACQUIRE_POLL_FLOOR`] to
//! [`ACQUIRE_POLL_CEILING`]: a short handover costs microseconds and a long
//! queue does not burn a core. The process-wide mutex and the file lock share
//! one [`Window`]: holder progress renews it, and a full window without
//! progress returns `StoreBusy` that names the lock path. Nothing restarts
//! that window except observed progress.
use std::fs::File;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::contention::Notice;
use crate::locking::{
    HeldWriteLock, open_write_lock_file, process_write_lock, write_lock_is_held, write_lock_key,
};
use crate::progress::Window;

const ACQUIRE_POLL_FLOOR: Duration = Duration::from_millis(1);
const ACQUIRE_POLL_CEILING: Duration = Duration::from_millis(25);

/// One acquisition's wait: its window, its backoff and its warn-level notices.
pub(crate) struct AcquireWait<'a> {
    path: &'a Path,
    context: &'a str,
    wait: Duration,
    window: Window,
    notice: Notice,
    backoff: Duration,
    waiting: bool,
}

impl<'a> AcquireWait<'a> {
    pub(crate) fn new(path: &'a Path, context: &'a str, wait: Duration) -> Self {
        Self {
            path,
            context,
            wait,
            window: Window::new(path, wait),
            notice: Notice::new(),
            backoff: ACQUIRE_POLL_FLOOR,
            waiting: false,
        }
    }

    /// Sleep before the next try, or end the wait as a typed pause.
    fn sleep_or_pause(&mut self, holder: &str) -> Result<()> {
        if !std::mem::replace(&mut self.waiting, true) {
            tracing::debug!(
                context = self.context,
                lock_path = %self.path.display(),
                holder,
                "Cozo write lock is held; waiting while the holder makes progress"
            );
            #[cfg(any(test, feature = "test-support"))]
            crate::busy_observer::notify(
                self.context,
                &format!(
                    "Cozo write lock at {} is held by {holder}; waiting for writer progress",
                    self.path.display()
                ),
            );
        }
        let Some(remaining) = self.window.remaining() else {
            tracing::warn!(
                context = self.context,
                lock_path = %self.path.display(),
                holder,
                no_progress_ms = self.wait.as_millis() as u64,
                "Cozo write lock holder made no progress; pausing the operation"
            );
            return Err(crate::busy::lock_window_busy(
                self.context,
                format!(
                    "Cozo write lock at {} was still held by {holder} after {}ms with no writer progress",
                    self.path.display(),
                    self.wait.as_millis()
                ),
            ));
        };
        if let Some(waited) = self.notice.due() {
            tracing::warn!(
                context = self.context,
                lock_path = %self.path.display(),
                holder,
                waited_ms = waited.as_millis() as u64,
                pause_after_ms = remaining.as_millis() as u64,
                "Cozo write lock still held; waiting while the holder makes progress"
            );
        }
        std::thread::sleep(self.backoff.min(remaining));
        self.backoff = (self.backoff * 2).min(ACQUIRE_POLL_CEILING);
        Ok(())
    }
}

/// Run `run` while holding the write lock for `path`, waiting while its
/// holder makes progress and pausing after `wait` without progress.
///
/// Callers that need an actual mutual exclusion window -- a read-then-reserve
/// compare-and-set that must not be interleaved -- use this rather than the
/// fail-fast lock, because losing that race is not recoverable by retrying.
///
/// * **Resumable.** A window that expires returns retryable `StoreBusy`,
///   naming the lock file. Successful statements and database/journal changes
///   renew it; it never caps `run` once the lock is held.
/// * **Re-entrant.** On Windows `LockFileEx` byte-range locks conflict between
///   handles *within one process*, so a thread that already owns this lock
///   runs `run` inline under the lock it holds (see [`HeldWriteLock`]).
/// * **Two-layer.** The process-wide mutex on the same canonical path orders
///   this process's threads, so only one of them contends for the OS lock.
pub(crate) fn with_write_lock_blocking<T>(
    path: &Path,
    context: &str,
    wait: Duration,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let key = write_lock_key(Some(path))
        .with_context(|| format!("{context}: resolve Cozo write lock {}", path.display()))?;
    if write_lock_is_held(&key) {
        tracing::trace!(
            context,
            lock_path = %path.display(),
            "reusing Cozo write lock already held by this thread"
        );
        return run();
    }
    let mut pending = AcquireWait::new(path, context, wait);
    let process_lock = process_write_lock(&key);
    let _process_guard = acquire_process_lock(&process_lock, &mut pending)?;
    let _held_lock = HeldWriteLock::enter(key);
    let file = open_write_lock_file(path, context)?;
    acquire_file_lock(file, &mut pending, run)
}

/// Batch callers keep their operation across a wait. This is the same single
/// no-progress window as [`with_write_lock_blocking`]: it continues while the
/// holder progresses and ends as one typed pause, never restarting by itself.
pub(crate) fn with_write_lock_resuming<T>(
    path: &Path,
    context: &str,
    wait: Duration,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_write_lock_blocking(path, context, wait, run)
}

/// Take the process-wide mutex for a lock key under the acquisition window.
///
/// `std::sync::Mutex` has no timed acquire, so this polls `try_lock`. A
/// poisoned mutex is recovered rather than propagated: the data is `()`, so a
/// panicking holder leaves nothing inconsistent behind.
fn acquire_process_lock<'a>(
    lock: &'a Mutex<()>,
    pending: &mut AcquireWait<'_>,
) -> Result<MutexGuard<'a, ()>> {
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => pending.sleep_or_pause("this process")?,
        }
    }
}

/// Poll the OS lock on `file`. Only `WouldBlock` is contention; any other
/// lock error is permanent and is returned at once with the lock path.
pub(crate) fn acquire_file_lock<T>(
    file: File,
    pending: &mut AcquireWait<'_>,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut lock = fd_lock::RwLock::new(file);
    let mut run = Some(run);
    loop {
        match lock.try_write() {
            Ok(_guard) => {
                tracing::trace!(
                    context = pending.context,
                    lock_path = %pending.path.display(),
                    "acquired blocking Cozo write lock"
                );
                let run = run
                    .take()
                    .expect("the blocking write lock body is taken exactly once");
                return run();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                pending.sleep_or_pause("another handle")?;
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "{}: Cozo write lock failed at {}",
                    pending.context,
                    pending.path.display()
                )));
            }
        }
    }
}
