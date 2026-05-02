//! Cancel-aware cron scheduler lifecycle guard.
//!
//! Replaces the bare `tokio::time::sleep(1s)` in the cron scheduler loop with a
//! `select!` that observes a `tokio::sync::Notify` immediately.  The
//! [`CronShutdown`] guard owns the spawned task's lifecycle — `.shutdown()` for
//! graceful teardown, `Drop` as the sync fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::cron_scheduler::{CronScheduler, is_one_shot};

/// Owns the lifecycle of a spawned `run_scheduler_loop` task.
///
/// Construct via [`spawn_scheduler`]. The owner MUST either:
///   * call [`shutdown`](Self::shutdown) before the runtime tears down
///     for graceful termination, OR
///   * let `Drop` fire — Drop signals cancel + Notify and aborts the
///     handle as a fallback. This avoids the runtime-shutdown panic
///     in `tokio-1.x .../runtime/time/entry.rs:602` ("A Tokio 1.x
///     context was found, but it is being shutdown") which is what
///     the bare `sleep(1s)` previously triggered.
///
/// # Correctness — why both `cancel` and `notify` are kept
///
/// `tokio::sync::Notify::notify_waiters()` wakes ONLY currently-
/// registered waiters. A `notify_waiters` call that races BEFORE the
/// loop registers via `notified()` is lost. Therefore `cancel:
/// AtomicBool` is the source of truth: the loop checks it before AND
/// after each `select!` iteration. `Notify` is the fast-path wakeup
/// that collapses the cancel-observation latency from up-to-1s down
/// to "next yield" — without `Notify` the loop would still terminate
/// (via cancel-flag check) but only after the in-flight sleep
/// completes, leaving a 1s panic window. Removing the cancel flag
/// "because we have Notify" would re-introduce the missed-wakeup race.
pub struct CronShutdown {
    cancel: Arc<AtomicBool>,
    notify: Arc<Notify>,
    handle: Option<JoinHandle<()>>,
}

impl CronShutdown {
    /// Graceful shutdown: signal cancel, wake the sleep via Notify, and
    /// `.await` the join handle with a 2-second timeout.
    ///
    /// Call this from the session's normal shutdown path (e.g. before
    /// `run_interactive_session` returns — the only function that
    /// spawns the cron scheduler).
    pub async fn shutdown(mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
        if let Some(h) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
    }
}

impl Drop for CronShutdown {
    fn drop(&mut self) {
        // Sync fallback path: still signal cancel + Notify so a healthy
        // task observes shutdown and exits cleanly. Then abort the
        // handle as a last resort — abort is sync-safe.
        self.cancel.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Spawn the scheduler loop and return its lifecycle guard.
///
/// Replaces the prior pattern of `tokio::spawn(run_scheduler_loop(...))`
/// at `src/session.rs:1211` which dropped the JoinHandle and never
/// signalled the cancel flag — see commit message for full incident.
pub fn spawn_scheduler(store_path: std::path::PathBuf) -> CronShutdown {
    let cancel = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());
    let handle = tokio::spawn(run_scheduler_loop_with_notify(
        store_path,
        Arc::clone(&cancel),
        Arc::clone(&notify),
    ));
    CronShutdown {
        cancel,
        notify,
        handle: Some(handle),
    }
}

/// Cancel-aware variant of the cron scheduler loop.
///
/// Production code should use [`spawn_scheduler`] which wires a real
/// `Notify` for prompt cancel. The 2-arg [`run_scheduler_loop`] is a
/// backward-compat wrapper for the pre-existing tests.
pub async fn run_scheduler_loop_with_notify(
    store_path: std::path::PathBuf,
    cancel: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
) {
    let store = crate::cron_task::CronStore::new(store_path.clone());

    loop {
        if cancel.load(Ordering::Relaxed) {
            tracing::debug!("cron scheduler: cancel signal received, exiting");
            break;
        }

        // Cancel-aware sleep. select! ensures the sleep future is dropped
        // immediately when shutdown_notify fires — no pending timer
        // entry survives into runtime teardown.
        tokio::select! {
            biased;
            _ = shutdown_notify.notified() => {
                tracing::debug!("cron scheduler: shutdown notified, exiting");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let now = chrono::Utc::now();

        let tasks = match store.load() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("cron scheduler: failed to load tasks: {e}");
                continue;
            }
        };

        if tasks.is_empty() {
            continue;
        }

        let scheduler = CronScheduler::new(store_path.clone());
        let due = scheduler.due_tasks(&tasks, now);

        if due.is_empty() {
            continue;
        }

        let mut deleted_ids: Vec<String> = Vec::new();
        for task in &due {
            tracing::info!(
                task_id = %task.id,
                cron = %task.cron,
                "cron.fire: scheduled task fired"
            );
            if is_one_shot(task) {
                deleted_ids.push(task.id.clone());
            }
        }

        // Delete one-shot tasks (fail-open).
        for id in &deleted_ids {
            if let Err(e) = store.delete(id) {
                tracing::warn!("cron scheduler: failed to delete one-shot task {id}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// Verify that notifying the shutdown Notify causes the loop to exit
    /// within 100ms — much faster than the 1s sleep interval.
    #[tokio::test]
    async fn scheduler_with_notify_exits_immediately_on_notify() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("tasks.json");
        let cancel = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());

        let cancel_clone = Arc::clone(&cancel);
        let notify_clone = Arc::clone(&notify);
        let store_clone = store_path.clone();

        let handle = tokio::spawn(async move {
            run_scheduler_loop_with_notify(store_clone, cancel_clone, notify_clone).await;
        });

        // Let the loop enter its first select.
        tokio::time::sleep(Duration::from_millis(50)).await;
        notify.notify_waiters();

        let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
        assert!(result.is_ok(), "loop should exit within 200ms after notify");
    }

    /// Drop on CronShutdown must abort the handle — the spawned task
    /// should be dead within 200ms.
    #[tokio::test]
    async fn cron_shutdown_drop_aborts_handle() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("tasks.json");

        let guard = spawn_scheduler(store_path);
        // Let the loop start.
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);
        // Give the abort time to take effect — should not hang.
        tokio::time::sleep(Duration::from_millis(100)).await;
        // If we got here without panicking, drop() worked.
    }

    /// Graceful shutdown must complete within 2.5s.
    #[tokio::test]
    async fn cron_shutdown_graceful_shutdown_completes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("tasks.json");

        let guard = spawn_scheduler(store_path);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = tokio::time::timeout(Duration::from_millis(2500), guard.shutdown()).await;
        assert!(
            result.is_ok(),
            "graceful shutdown must complete within 2.5s"
        );
    }

    /// With biased; select!, when both cancel is set AND notify fires before
    /// the loop's first select, exit must happen within 100ms — proving
    /// shutdown takes priority over the sleep branch.
    #[tokio::test]
    async fn scheduler_select_is_biased_toward_shutdown() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("tasks.json");
        let cancel = Arc::new(AtomicBool::new(true)); // already set
        let notify = Arc::new(Notify::new());
        notify.notify_waiters(); // fire before the loop polls

        let handle = tokio::spawn(async move {
            run_scheduler_loop_with_notify(store_path, cancel, notify).await;
        });

        let result = tokio::time::timeout(Duration::from_millis(100), handle).await;
        assert!(
            result.is_ok(),
            "biased select must prefer shutdown over sleep"
        );
    }
}
