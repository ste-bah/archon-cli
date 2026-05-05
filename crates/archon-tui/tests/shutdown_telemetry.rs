use std::time::Duration;

use archon_tui::observability::{
    log_alive_tasks_after_cancel, reset_task_registry_for_tests, spawn_named, task_snapshots,
};

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
async fn log_alive_tasks_after_cancel_reports_all_finished_when_clean() {
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
