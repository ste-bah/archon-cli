use super::FocusedTestPlan;
use std::path::PathBuf;

tokio::task_local! { pub(super) static READ_SET_PATH: PathBuf; }
tokio::task_local! { pub(super) static FOCUSED_TESTS: FocusedTestPlan; }

/// Bind the read-set sidecar to the task that constructs its guard.
pub async fn scope_read_set<T>(path: PathBuf, work: impl std::future::Future<Output = T>) -> T {
    READ_SET_PATH.scope(path, work).await
}

/// Bind the focused checks to the task that constructs its guard.
pub async fn scope_focused_tests<T>(
    plan: FocusedTestPlan,
    work: impl std::future::Future<Output = T>,
) -> T {
    FOCUSED_TESTS.scope(plan, work).await
}
