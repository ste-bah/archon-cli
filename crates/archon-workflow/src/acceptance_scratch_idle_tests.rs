//! Issue 356: a site's limit bounds inactivity, never active elapsed work.
use super::*;

async fn active(text: &str) {
    let temp = tempfile::tempdir().unwrap();
    let command = AuthorizedCommand::for_test(text, TrustedCwd::ProjectRoot);
    let result = run_at(
        &site(temp.path(), false, 1),
        "check",
        &command,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert!(
        result.operational_error.is_none(),
        "{:?}",
        result.operational_error
    );
    assert_eq!(result.exit_code, Some(0));
}

#[tokio::test]
async fn issue356_check_output_renews() {
    active("for i in 1 2 3 4 5 6; do printf x; sleep 0.3; done").await;
}
#[tokio::test]
async fn issue356_check_stderr_renews() {
    active("for i in 1 2 3 4 5 6; do printf x >&2; sleep 0.3; done").await;
}
#[tokio::test]
async fn issue356_check_descendant_cpu_renews() {
    // The shell waits silently while its descendant does real work without output.
    active("perl -MTime::HiRes=time -e '$end=time()+2; while(time()<$end) {$n++}'").await;
}

#[tokio::test]
async fn issue356_check_stalls_from_last_output() {
    for destination in ["", ">&2", "1>&2"] {
        let temp = tempfile::tempdir().unwrap();
        let text =
            format!("for i in 1 2 3 4; do printf x {destination}; sleep 0.3; done; sleep 30");
        let command = AuthorizedCommand::for_test(&text, TrustedCwd::ProjectRoot);
        let started = tokio::time::Instant::now();
        let result = run_at(
            &site(temp.path(), false, 1),
            "check",
            &command,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(result.operational_error.as_deref(), Some(CHECK_TIMED_OUT));
        assert!(
            started.elapsed() >= Duration::from_millis(1_700),
            "silence is measured from the last output, not spawn"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a real stall must stop"
        );
    }
}
