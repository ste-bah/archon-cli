//! Issue 356: child output renews supervision, even without saved-unit credit.
use super::*;

async fn active(body: &str) {
    let temp = tempfile::tempdir().unwrap();
    let mut request = command(script(temp.path(), "idle-window", body));
    request.timeout_secs = 1;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert!(
        !output.timed_out,
        "active output was stopped by a total clock"
    );
    assert_eq!(output.exit_code, Some(0));
}

#[tokio::test]
async fn issue356_host_stdout_renews() {
    active("for i in 1 2 3 4 5 6; do printf x; sleep 0.3; done").await;
}
#[tokio::test]
async fn issue356_host_stderr_renews() {
    active("for i in 1 2 3 4 5 6; do printf x >&2; sleep 0.3; done").await;
}
#[tokio::test]
async fn issue356_host_saved_progress_renews() {
    active(
        "for i in 1 2 3 4 5 6; do printf 'archon-host-progress: %s\\n' \"$i\" >&2; sleep 0.3; done",
    )
    .await;
}
