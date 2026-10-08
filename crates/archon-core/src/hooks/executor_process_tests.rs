use super::*;

#[test]
fn stdin_timeout_preserves_cleanup_failure_context() {
    let error = combine_cleanup_error(
        RunError::Timeout("stdin write"),
        Some("fixture cleanup failure".to_string()),
    );

    assert_eq!(
        error.to_string(),
        "I/O error: timed out: no progress during stdin write; process cleanup failed: fixture cleanup failure"
    );
}

#[tokio::test]
async fn an_expired_budget_outranks_a_phase_that_already_finished() {
    let deadline = NoProgressWindow::new(Duration::ZERO);

    // An expired idle window must outrank even a ready exit status.
    let phase = within_window(&deadline, "process wait", std::future::ready(0_i32)).await;

    assert!(
        matches!(phase, Err(RunError::Timeout("process wait"))),
        "expected a timeout once the budget is gone, got {phase:?}"
    );
}

#[tokio::test]
async fn a_hook_with_no_work_budget_cannot_report_a_clean_run() {
    let dir = tempfile::tempdir().unwrap();

    // `true` exits 0 immediately, so every phase after the spawn is ready on
    // its first poll. With a zero work budget the only honest verdict is a
    // timeout: the spawn succeeded, the hook was never entitled to run.
    let result = run_command("exit 0", b"{}", dir.path(), "deadline", "PreToolUse", 0).await;

    assert!(
        matches!(result, Err(RunError::Timeout(_))),
        "expected a timeout, got {result:?}"
    );
}

#[tokio::test]
async fn stdout_only_truncation_stays_within_the_exact_shared_bound() {
    let stdout = truncated_pipe(b"stdout", 3).await;
    let output = CommandOutput::from_pipes(0, stdout, empty_pipe());

    assert_output_bound(&output);
    assert_eq!(marker_count(&output), 1);
    assert!(output.stdout.contains(TRUNCATION_MARKER));
    assert!(!output.stderr.contains(TRUNCATION_MARKER));
}

#[tokio::test]
async fn stderr_only_truncation_stays_within_the_exact_shared_bound() {
    let stderr = truncated_pipe(b"stderr", 3).await;
    let output = CommandOutput::from_pipes(0, empty_pipe(), stderr);

    assert_output_bound(&output);
    assert_eq!(marker_count(&output), 1);
    assert!(!output.stdout.contains(TRUNCATION_MARKER));
    assert!(output.stderr.contains(TRUNCATION_MARKER));
}

#[tokio::test]
async fn simultaneous_truncation_has_one_combined_marker_within_bound() {
    let budget = Arc::new(AtomicUsize::new(6));
    let (mut out_writer, out_reader) = tokio::io::duplex(64);
    let (mut err_writer, err_reader) = tokio::io::duplex(64);
    let mut stdout_task = drain_pipe(
        Some(out_reader),
        Arc::clone(&budget),
        NoProgressWindow::new(Duration::from_secs(60)),
    );
    let mut stderr_task = drain_pipe(
        Some(err_reader),
        budget,
        NoProgressWindow::new(Duration::from_secs(60)),
    );
    tokio::join!(
        async { out_writer.write_all(b"stdout").await.unwrap() },
        async { err_writer.write_all(b"stderr").await.unwrap() }
    );
    drop(out_writer);
    drop(err_writer);

    let deadline = NoProgressWindow::new(Duration::from_secs(1));
    let (stdout, stderr) = join_pipes(&deadline, &mut stdout_task, &mut stderr_task)
        .await
        .unwrap();
    let output = CommandOutput::from_pipes(0, stdout, stderr);

    assert_output_bound(&output);
    assert_eq!(marker_count(&output), 1);
}

async fn truncated_pipe(bytes: &[u8], budget: usize) -> PipeOutput {
    let (mut writer, reader) = tokio::io::duplex(64);
    let task = drain_pipe(
        Some(reader),
        Arc::new(AtomicUsize::new(budget)),
        NoProgressWindow::new(Duration::from_secs(60)),
    );
    writer.write_all(bytes).await.unwrap();
    drop(writer);
    task.await.unwrap()
}

fn assert_output_bound(output: &CommandOutput) {
    assert!(total_output_bytes(output) <= HOOK_OUTPUT_BYTES);
}

fn marker_count(output: &CommandOutput) -> usize {
    output.stdout.matches(TRUNCATION_MARKER).count()
        + output.stderr.matches(TRUNCATION_MARKER).count()
}

fn empty_pipe() -> PipeOutput {
    PipeOutput {
        bytes: Vec::new(),
        truncated: false,
        read_error: None,
    }
}
