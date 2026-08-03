use super::windows_direct_child_drains_large_output;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_raw_child_drains_large_output() {
    windows_direct_child_drains_large_output(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_raw_child_drains_large_output() {
    windows_direct_child_drains_large_output(false, false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_job_object_child_drains_large_output() {
    windows_direct_child_drains_large_output(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_job_object_child_drains_large_output() {
    windows_direct_child_drains_large_output(true, false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_raw_shell_child_drains_large_output() {
    windows_direct_child_drains_large_output(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_raw_shell_child_drains_large_output() {
    windows_direct_child_drains_large_output(false, true, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_job_object_shell_child_drains_large_output() {
    windows_direct_child_drains_large_output(true, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_job_object_shell_child_drains_large_output() {
    windows_direct_child_drains_large_output(true, true, true).await;
}
