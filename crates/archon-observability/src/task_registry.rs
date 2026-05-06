//! Named Tokio task registry for shutdown forensics.
//!
//! Long-lived Archon background tasks should use [`spawn_named`] instead of
//! raw `tokio::spawn`. The registry keeps each task's [`tokio::task::AbortHandle`]
//! plus spawn time so shutdown can report which tasks are still alive after
//! cooperative cancellation has been signalled.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::task::{AbortHandle, JoinHandle};

#[derive(Clone, Debug)]
struct TrackedTask {
    name: String,
    abort: AbortHandle,
    spawned_at: Instant,
}

/// Snapshot of a task recorded in the shutdown registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSnapshot {
    /// Human-readable task name supplied to [`spawn_named`].
    pub name: String,
    /// How long the task has been alive.
    pub elapsed: Duration,
    /// Whether Tokio reports the task as finished.
    pub is_finished: bool,
}

/// Spawn a Tokio task and record it under a human-readable name.
///
/// Use this for session-long or detached background tasks. Short request/response
/// tasks may still use raw `tokio::spawn` when a local owner awaits them.
pub fn spawn_named<F>(name: impl Into<String>, fut: F) -> JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let name = name.into();
    let handle = tokio::spawn(fut);
    register_abort_handle(name, handle.abort_handle());
    handle
}

/// Spawn blocking work and record it under a human-readable name.
///
/// Tokio cannot abort blocking work once it has started running, so this is
/// especially useful for identifying CPU-heavy shutdown survivors.
pub fn spawn_blocking_named<F, R>(name: impl Into<String>, func: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let name = name.into();
    let handle = tokio::task::spawn_blocking(func);
    register_abort_handle(name, handle.abort_handle());
    handle
}

/// Register an already-spawned task by name.
///
/// Prefer [`spawn_named`] when possible. This helper exists for lifecycle
/// guards that need to spawn internally but still want shutdown visibility.
pub fn register_abort_handle(name: impl Into<String>, abort: AbortHandle) {
    registry()
        .lock()
        .expect("shutdown task registry lock poisoned")
        .push(TrackedTask {
            name: name.into(),
            abort,
            spawned_at: Instant::now(),
        });
}

/// Return a snapshot of every task currently recorded.
pub fn task_snapshots() -> Vec<TaskSnapshot> {
    registry()
        .lock()
        .expect("shutdown task registry lock poisoned")
        .iter()
        .map(|task| TaskSnapshot {
            name: task.name.clone(),
            elapsed: task.spawned_at.elapsed(),
            is_finished: task.abort.is_finished(),
        })
        .collect()
}

/// Sleep for `grace_period`, log alive tasks, and return the alive snapshot.
///
/// This function is intentionally synchronous so it can run at the very end of
/// shutdown, after cooperative cancellation has already been triggered.
pub fn log_alive_tasks_after_cancel(grace_period: Duration) -> Vec<TaskSnapshot> {
    std::thread::sleep(grace_period);
    let alive: Vec<TaskSnapshot> = task_snapshots()
        .into_iter()
        .filter(|task| !task.is_finished)
        .collect();
    if alive.is_empty() {
        tracing::info!("shutdown: all spawned tasks completed cleanly");
    } else {
        tracing::warn!(
            count = alive.len(),
            tasks = ?alive,
            "shutdown: tasks still alive after cooperative cancel"
        );
    }
    alive
}

/// Abort every task that is still alive in the registry.
///
/// This is a last-ditch shutdown fence. It will stop ordinary async tasks at
/// their next cancellation point. A CPU-bound task that never yields will remain
/// visible in a follow-up [`log_alive_tasks_after_cancel`] call.
pub fn abort_alive_tasks() -> usize {
    let tasks = registry()
        .lock()
        .expect("shutdown task registry lock poisoned");
    let mut aborted = 0usize;
    for task in tasks.iter().filter(|task| !task.abort.is_finished()) {
        task.abort.abort();
        aborted += 1;
    }
    if aborted > 0 {
        tracing::warn!(count = aborted, "shutdown: aborted alive spawned tasks");
    }
    aborted
}

/// Clear the global task registry.
///
/// This is only intended for tests; production code should keep the registry
/// process-long so shutdown can inspect every detached task.
#[doc(hidden)]
pub fn reset_task_registry_for_tests() {
    registry()
        .lock()
        .expect("shutdown task registry lock poisoned")
        .clear();
}

fn registry() -> &'static Mutex<Vec<TrackedTask>> {
    static REGISTRY: OnceLock<Mutex<Vec<TrackedTask>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_named_records_task_in_registry() {
        reset_task_registry_for_tests();
        let handle = spawn_named("test-task", async {});
        handle.await.unwrap();

        let snapshots = task_snapshots();
        let task = snapshots
            .iter()
            .find(|task| task.name == "test-task")
            .expect("test-task should be registered");
        assert!(task.is_finished);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn log_alive_tasks_after_cancel_reports_clean_shutdown() {
        reset_task_registry_for_tests();
        let handle = spawn_named("short-task", async {});
        handle.await.unwrap();

        let alive = log_alive_tasks_after_cancel(Duration::from_millis(10));
        assert!(alive.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn log_alive_tasks_after_cancel_reports_stuck_task() {
        reset_task_registry_for_tests();
        let handle = spawn_named("stuck-task", async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let alive = log_alive_tasks_after_cancel(Duration::from_millis(10));
        assert!(alive.iter().any(|task| task.name == "stuck-task"));

        handle.abort();
        let _ = handle.await;
    }
}
