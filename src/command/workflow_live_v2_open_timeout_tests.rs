//! Native pre-header and error-body timeouts through the real executor.
#![allow(clippy::await_holding_lock)]
use super::recovery_http_tests::endpoint_mode;
use super::recovery_tests::{harness_with_inactivity, request};
use archon_workflow::llm_retry::run_agent_with_transient_retry;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

async fn recover(stalled_body: bool) {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let (provider, calls, server) =
            endpoint_mode(503, "AC-X-001", alternate, !alternate, true, stalled_body).await;
        let client = harness_with_inactivity(provider, root.path(), 30, 1);
        let retries = AtomicUsize::new(0);
        let result = archon_tools::subagent_dispatch_clock::within_named(
            Duration::from_secs(1),
            run_agent_with_transient_retry(&client, request(), |_| {
                retries.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            }),
        )
        .await;
        server.abort();
        assert!(
            result.as_ref().is_ok_and(|result| result.is_ok()),
            "native timeout reached caller: {result:?}"
        );
        assert_eq!(retries.load(Ordering::SeqCst), 1);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        if stalled_body {
            assert!(
                calls[1] - calls[0] >= Duration::from_secs(90),
                "error-body timeout lost the response header"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_open_timeout_recovers_before_response_headers() {
    recover(false).await;
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_stalled_error_body_preserves_retry_after() {
    recover(true).await;
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_session_deadline_survives_partial_retries_without_outer_clock() {
    use super::recovery_tests::{Fault, harness, transport};
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(Fault::Partial);
    let client = harness(provider.clone(), root.path(), 1);
    let mut call = request();
    call.timeout_secs = Some(60);
    let start = tokio::time::Instant::now();
    let work = run_agent_with_transient_retry(&client, call, |_| async { Ok(()) });
    let result = tokio::select! {
        result = work => result,
        () = tokio::time::sleep(Duration::from_secs(120)) => panic!("fresh sessions renewed the deadline without an outer author clock"),
    };
    assert!(
        result.is_err(),
        "expected the existing session deadline: {result:?}"
    );
    assert!(start.elapsed() >= Duration::from_secs(60));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("timed out after 60s")
    );
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_error_body_disconnect_preserves_status_and_retry_after() {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let (provider, calls, server) =
            super::recovery_http_tests::endpoint_disconnect(alternate).await;
        let client = harness_with_inactivity(provider, root.path(), 30, 1);
        let result = run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }).await;
        server.abort();
        assert!(
            result.is_ok(),
            "body disconnect lost the response status: {result:?}"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1] - calls[0] >= Duration::from_secs(90),
            "body disconnect lost Retry-After"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_rate_limit_without_header_uses_shared_initial_delay() {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let (provider, calls, server) =
            super::recovery_http_tests::endpoint_without_hint(alternate).await;
        let client = harness_with_inactivity(provider, root.path(), 30, 1);
        let result = run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }).await;
        server.abort();
        assert!(result.is_ok(), "{result:?}");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let delay = calls[1] - calls[0];
        assert!(delay >= Duration::from_millis(500));
        assert!(
            delay < Duration::from_secs(1),
            "client default leaked into the retry policy: {delay:?}"
        );
    }
}
