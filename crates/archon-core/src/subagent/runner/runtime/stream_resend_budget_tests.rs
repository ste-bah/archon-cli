use super::*;

#[tokio::test]
async fn quick_unanswered_resends_inside_the_window_never_stop_the_round() {
    let mut budget = ResendBudget::new(Duration::from_secs(3600));
    for _ in 0..50 {
        assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
    }
}

#[tokio::test]
async fn unanswered_resends_stop_after_a_whole_window_with_the_stall_marker() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    for _ in 0..STREAM_RETRIES {
        assert!(budget.failed(FailedAttempt::Unanswered("idle")).is_ok());
    }
    let stop = budget
        .failed(FailedAttempt::Unanswered("idle"))
        .expect_err("a whole window without an answer stops the round");
    assert!(
        stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{stop}"
    );
    assert!(stop.contains("stream retry exhausted"), "{stop}");
}

#[tokio::test]
async fn an_answer_starts_a_new_window() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    budget.answered();
    for _ in 0..10 {
        assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
    }
}

/// Fails before the fix: awake silence from before a sleep counted, so
/// a few quick failures after the wake stopped the round.
#[tokio::test]
async fn a_sleep_starts_a_new_window_at_the_wake() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    budget.note_expiry(IdleExpired { slept: true });
    for _ in 0..10 {
        assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
    }
    let mut awake = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    awake.note_expiry(IdleExpired { slept: false });
    assert!((0..10).any(|_| awake.failed(FailedAttempt::Unanswered("open")).is_err()));
}

#[test]
fn unanswered_backoff_doubles_up_to_its_cap() {
    let (first, cap) = UNANSWERED_BACKOFF;
    assert_eq!(unanswered_backoff(1), first);
    assert_eq!(unanswered_backoff(2), first * 2);
    assert_eq!(unanswered_backoff(60), cap);
}

/// Fails before the fix: the stop said "no answer" and dropped the
/// provider's status and body.
#[tokio::test]
async fn the_stop_names_the_last_provider_error_bounded_and_redacted() {
    let mut budget = ResendBudget::new(Duration::from_millis(20));
    let body = format!(
        "model not loaded api_key=sk-ant-secret123 {}",
        "x".repeat(900)
    );
    let server = || {
        anyhow::Error::new(LlmError::Server {
            status: 500,
            message: body.clone(),
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    let stop = (0..10)
        .find_map(|_| {
            budget
                .open_failed(server(), "first open could not open")
                .err()
        })
        .expect("a whole window of 500s stops the round")
        .to_string();
    assert!(
        stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{stop}"
    );
    assert!(stop.contains("no successful answer"), "{stop}");
    assert!(
        stop.contains("server error (500): model not loaded"),
        "{stop}"
    );
    assert!(!stop.contains("sk-ant-secret123"), "{stop}");
    assert!(stop.contains("[truncated:") && stop.len() < 900, "{stop}");
}

/// Fails before the fix: a long Retry-After failed the call.
#[tokio::test]
async fn a_rate_limit_is_waited_inside_the_window_and_paused_past_it() {
    let limited = |secs| {
        anyhow::Error::new(LlmError::RateLimited {
            retry_after_secs: secs,
            from_provider: true,
        })
    };
    let mut budget = ResendBudget::new(Duration::from_secs(600));
    let wait = budget.open_failed(limited(90), "first open could not open");
    assert_eq!(
        wait.expect("served inside the window"),
        Duration::from_secs(90)
    );
    let stop = budget
        .open_failed(limited(8_004), "first open could not open")
        .expect_err("past the window it stops now")
        .to_string();
    assert!(
        stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{stop}"
    );
    assert!(stop.contains("retry after 8004s (at 20"), "{stop}");
    assert!(stop.contains("resume after that time"), "{stop}");
}

/// Fails before the fix (item 5): a rejected open came back with the raw
/// provider body, secrets included.
#[tokio::test]
async fn a_rejected_open_ends_the_round_at_once_redacted() {
    let mut budget = ResendBudget::new(Duration::from_secs(600));
    let error = budget
        .open_failed(
            anyhow::Error::new(LlmError::Server {
                status: 400,
                message: "invalid body api_key=sk-ant-secret123".into(),
            }),
            "first open could not open",
        )
        .expect_err("a 4xx is not retried");
    let text = error.to_string();
    assert!(
        text.starts_with("server error (400): invalid body"),
        "{text}"
    );
    assert!(!text.contains("sk-ant-secret123"), "{text}");
    assert!(
        matches!(
            error.downcast_ref::<LlmError>(),
            Some(LlmError::Server { status: 400, .. })
        ),
        "the variant is kept"
    );
    assert_eq!(budget.failures(), 0);
}

/// Fails before the fix (item 6): more than three incomplete streams failed
/// the round with no pause marker, however quickly they came.
#[tokio::test]
async fn incomplete_streams_resend_inside_the_window_and_pause_past_it() {
    let mut budget = ResendBudget::new(Duration::from_secs(3600));
    for _ in 0..10 {
        assert!(budget.failed(FailedAttempt::Incomplete).is_ok());
    }
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    budget.note_stream_error("protocol", "stream ended before message_stop");
    let stop = (0..10)
        .find_map(|_| budget.failed(FailedAttempt::Incomplete).err())
        .expect("a whole window without a complete answer stops the round");
    assert!(
        stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{stop}"
    );
    assert!(stop.contains("no complete answer"), "{stop}");
    assert!(stop.contains("stream ended before message_stop"), "{stop}");
}

/// Fails before the fix (item 2): an answer kept the old error, so a later
/// silent stall named a 500 that was no longer the cause.
#[tokio::test]
async fn an_answer_clears_the_last_provider_error() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    let server = anyhow::Error::new(LlmError::Server {
        status: 500,
        message: "model not loaded".into(),
    });
    assert!(
        budget
            .open_failed(server, "first open could not open")
            .is_ok()
    );
    budget.answered();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let stop = (0..10)
        .find_map(|_| {
            budget
                .failed(FailedAttempt::Unanswered("stream idle timeout"))
                .err()
        })
        .expect("a whole window of silence stops the round");
    assert!(stop.contains("no answer from the provider"), "{stop}");
    assert!(!stop.contains("model not loaded"), "{stop}");
}

/// Fails before the fix (item 3): a mid-stream `connection reset` was in no
/// stop text.
#[tokio::test]
async fn the_stop_names_a_mid_stream_transport_error() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(40)).await;
    budget.note_stream_error("network", "connection reset");
    let stop = (0..10)
        .find_map(|_| {
            budget
                .failed(FailedAttempt::Unanswered("transport error"))
                .err()
        })
        .expect("a whole window stops the round");
    assert!(
        stop.contains("last provider error: network: connection reset"),
        "{stop}"
    );
}

/// Fails before the fix (item 4): a default wait read as the provider's.
#[tokio::test]
async fn a_default_rate_limit_wait_is_not_called_the_providers() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    let limited = anyhow::Error::new(LlmError::RateLimited {
        retry_after_secs: 60,
        from_provider: false,
    });
    let stop = budget
        .open_failed(limited, "first open could not open")
        .expect_err("past the window it stops now")
        .to_string();
    assert!(stop.contains("default wait is 60s"), "{stop}");
    assert!(!stop.contains("provider asked"), "{stop}");
}

/// A stream that opens clears the old open error, but is not progress: a
/// stream that opens and resets again and again still reaches the window.
#[tokio::test]
async fn an_open_clears_the_old_error_but_does_not_renew_the_window() {
    let mut budget = ResendBudget::new(Duration::from_millis(30));
    let server = anyhow::Error::new(LlmError::Server {
        status: 500,
        message: "model not loaded".into(),
    });
    assert!(
        budget
            .open_failed(server, "first open could not open")
            .is_ok()
    );
    tokio::time::sleep(Duration::from_millis(40)).await;
    budget.opened();
    let stop = budget
        .failed(FailedAttempt::Unanswered("stream idle timeout"))
        .and_then(|_| budget.failed(FailedAttempt::Unanswered("stream idle timeout")))
        .and_then(|_| budget.failed(FailedAttempt::Unanswered("stream idle timeout")))
        .expect_err("the open did not renew the window");
    assert!(!stop.contains("model not loaded"), "{stop}");
}
