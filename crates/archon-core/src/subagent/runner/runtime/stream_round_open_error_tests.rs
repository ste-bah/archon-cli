// Issue 364 round 4: an open that fails names its real cause, a request the
// provider rejected fails fast, and a long rate limit pauses with its time.

async fn run_wake(runner: SubagentRunner, limit_secs: u64) -> String {
    tokio::time::timeout(
        std::time::Duration::from_secs(limit_secs),
        runner.run("work"),
    )
    .await
    .expect("the round must end")
    .expect_err("the round stops")
    .to_string()
}

/// Fails before the fix: the stop said "no answer ... (last: first open could
/// not open)" and the 500 and its body were in no text and no log.
#[tokio::test]
async fn a_first_open_that_always_answers_500_names_the_500_in_the_stop() {
    let opens = Arc::new(AtomicU32::new(0));
    let script = vec![WakeOpen::ServerError("model not loaded")];
    let error = run_wake(wake_runner(script, &opens, 1), 20).await;
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(error.contains("no successful answer"), "{error}");
    assert!(error.contains("first open could not open"), "{error}");
    assert!(
        error.contains("server error (500): model not loaded"),
        "{error}"
    );
}

/// Fails before the fix: the round-2 resend arm dropped the error too.
#[tokio::test]
async fn a_resend_that_always_answers_500_names_the_500_in_the_stop() {
    let opens = Arc::new(AtomicU32::new(0));
    let script = vec![WakeOpen::Silent, WakeOpen::ServerError("model not loaded")];
    let error = run_wake(wake_runner(script, &opens, 1), 20).await;
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(
        error.contains("server error (500): model not loaded"),
        "{error}"
    );
}

/// The production wrapper, with no waits between its attempts.
fn retried_wake_runner(script: Vec<WakeOpen>, opens: &Arc<AtomicU32>) -> SubagentRunner {
    let policy = archon_llm::RetryPolicy {
        max_attempts: 3,
        initial_backoff: std::time::Duration::from_millis(1),
        max_backoff: std::time::Duration::from_millis(1),
        multiplier: 1.0,
        jitter: false,
    };
    let inner: Arc<dyn LlmProvider> = wake_provider(script, opens);
    runner_over(
        Arc::new(archon_llm::RetryProvider::new(inner, policy)),
        3_600,
    )
}

/// Fails before the fix: a rejected request before any content came back as
/// `Http`, read as retryable, and backed off for a whole window.
#[tokio::test]
async fn a_rejected_request_before_content_fails_the_round_at_once() {
    for error_type in [
        "invalid_request_error",
        "authentication_error",
        "permission_error",
        "not_found_error",
    ] {
        let opens = Arc::new(AtomicU32::new(0));
        let runner = retried_wake_runner(vec![WakeOpen::ErrorEvent(error_type)], &opens);
        let error = run_wake(runner, 5).await;
        assert!(
            !error.contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
            "{error}"
        );
        assert!(error.contains(&format!("({error_type})")), "{error}");
        assert_eq!(opens.load(Ordering::SeqCst), 1, "{error_type}: no retry");
    }
}

/// A 5xx-class error before content is still retried and then recovers.
#[tokio::test]
async fn a_server_error_before_content_is_retried_until_it_answers() {
    // Fails before the fix (round 4): an unknown type, `parse_error` and
    // `timeout_error` failed fast as a fake `Server 400`.
    for error_type in [
        "api_error",
        "overloaded_error",
        "rate_limit_error",
        "parse_error",
        "timeout_error",
        "some_new_type",
    ] {
        let opens = Arc::new(AtomicU32::new(0));
        let mut script = vec![WakeOpen::ErrorEvent(error_type); 4];
        script.push(WakeOpen::Answers("recovered"));
        let runner = retried_wake_runner(script, &opens);
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), runner.run("w"))
            .await
            .expect("the round must end");
        assert_eq!(outcome.expect(error_type), "recovered");
    }
}

/// Fails before the fix: a Retry-After past the window failed the call.
#[tokio::test]
async fn a_rate_limit_past_the_window_pauses_with_the_retry_time() {
    let opens = Arc::new(AtomicU32::new(0));
    let runner = wake_runner(vec![WakeOpen::RateLimited(8_004)], &opens, 600);
    let error = run_wake(runner, 5).await;
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(error.contains("retry after 8004s (at "), "{error}");
    assert!(error.contains("rate limited: retry after 8004s"), "{error}");
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

/// Fails before the fix (round 5 item 3): a mid-stream transport error was
/// in no stop text.
#[tokio::test]
async fn a_stream_that_keeps_resetting_names_the_reset_in_the_stop() {
    let opens = Arc::new(AtomicU32::new(0));
    let runner = wake_runner(vec![WakeOpen::ErrorEvent("network")], &opens, 1);
    let error = run_wake(runner, 20).await;
    assert!(
        error.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(
        error.contains("last provider error: network: network from the provider"),
        "{error}"
    );
}
