use super::*;
use archon_llm::provider::LlmError;

#[test]
fn overload_backoff_doubles_then_caps_without_changing_attempt_budget() {
    let error = anyhow::Error::new(LlmError::Overloaded {
        status: Some(529),
        retry_after_secs: None,
    });
    let delays: Vec<_> = (1..=8)
        .map(|attempt| pipeline_attempt_retry_delay(attempt, &error).as_secs())
        .collect();
    assert_eq!(delays, [10, 20, 40, 80, 160, 300, 300, 300]);
    assert_eq!(PIPELINE_MAX_ATTEMPTS, 3);
}

#[test]
fn provider_retry_after_seconds_are_honoured_for_rate_limits_and_overload() {
    let rate_limited = anyhow::Error::new(LlmError::RateLimited {
        retry_after_secs: 37,
        from_provider: true,
        pipeline_retry_after_secs: None,
    });
    let overloaded = anyhow::Error::new(LlmError::Overloaded {
        status: Some(529),
        retry_after_secs: Some(53),
    });
    let pipeline_hint_only = anyhow::Error::new(LlmError::RateLimited {
        retry_after_secs: 0,
        from_provider: false,
        pipeline_retry_after_secs: Some(120),
    });
    assert_eq!(
        pipeline_attempt_retry_delay(1, &rate_limited),
        Duration::from_secs(37)
    );
    assert_eq!(
        pipeline_attempt_retry_delay(1, &overloaded),
        Duration::from_secs(53)
    );
    assert_eq!(
        pipeline_attempt_retry_delay(1, &pipeline_hint_only),
        Duration::from_secs(120)
    );
    let huge_hint = anyhow::Error::new(LlmError::Overloaded {
        status: Some(529),
        retry_after_secs: Some(86_400 * 20),
    });
    assert_eq!(
        pipeline_attempt_retry_delay(1, &huge_hint),
        Duration::from_secs(300)
    );
}

#[test]
fn display_text_cannot_spoof_provider_retry_policy() {
    let spoofed = anyhow::anyhow!("operation returned 529 but was invalid");
    assert_eq!(
        pipeline_attempt_retry_delay(1, &spoofed),
        Duration::from_millis(250)
    );
    assert!(!is_retryable_pipeline_attempt_error(&spoofed));
    let ordinary_503 = anyhow::Error::new(LlmError::Server {
        status: 503,
        message: "unavailable".into(),
        pipeline_retry_after_secs: None,
    });
    assert_eq!(
        pipeline_attempt_retry_delay(1, &ordinary_503),
        Duration::from_millis(250)
    );
    assert!(!is_pipeline_overload_error(&ordinary_503));
}

#[test]
fn typed_server_429_and_529_use_overload_policy_while_503_stays_ordinary() {
    for status in [429, 529] {
        let error = anyhow::Error::new(LlmError::Server {
            status,
            message: "transient server response".into(),
            pipeline_retry_after_secs: None,
        });
        assert!(is_retryable_pipeline_attempt_error(&error));
        assert!(is_pipeline_overload_error(&error));
        assert_eq!(
            pipeline_attempt_retry_delay(1, &error),
            Duration::from_secs(10)
        );
    }

    let error = anyhow::Error::new(LlmError::Server {
        status: 503,
        message: "temporarily unavailable".into(),
        pipeline_retry_after_secs: None,
    });
    assert!(is_retryable_pipeline_attempt_error(&error));
    assert!(!is_pipeline_overload_error(&error));
    assert_eq!(
        pipeline_attempt_retry_delay(1, &error),
        Duration::from_millis(250)
    );
}

#[test]
fn overloaded_503_uses_ordinary_retry_delay_even_with_provider_hint() {
    let error = anyhow::Error::new(LlmError::Overloaded {
        status: Some(503),
        retry_after_secs: Some(90),
    });
    assert!(is_retryable_pipeline_attempt_error(&error));
    assert!(!is_pipeline_overload_error(&error));
    assert_eq!(
        pipeline_attempt_retry_delay(1, &error),
        Duration::from_millis(250)
    );
}

#[test]
fn overload_text_fallback_requires_exact_provider_error_code_tokens() {
    for message in [
        "provider reported xoverloaded_errorx",
        "request for model gpt-529 was invalid",
        "request for model 529 was invalid",
        "status 429 from provider",
        "status 529 from provider",
    ] {
        let error = anyhow::anyhow!("{message}");
        assert!(!is_pipeline_overload_error(&error), "{message}");
        assert_eq!(
            pipeline_attempt_retry_delay(1, &error),
            Duration::from_millis(250),
            "{message}"
        );
    }
    for code in [
        "overloaded_error",
        "server_is_overloaded",
        "rate_limit_error",
    ] {
        let error = anyhow::anyhow!("provider returned {code}");
        assert!(is_pipeline_overload_error(&error), "{code}");
        assert_eq!(pipeline_attempt_retry_delay(1, &error).as_secs(), 10);
    }
}

#[test]
fn non_overload_errors_keep_the_existing_retry_delays() {
    let error = anyhow::anyhow!("HTTP error: 503 service unavailable");
    let delays: Vec<_> = (1..=4)
        .map(|attempt| pipeline_attempt_retry_delay(attempt, &error).as_millis())
        .collect();
    assert_eq!(delays, [250, 500, 1_000, 2_000]);
}

#[tokio::test]
async fn runner_sleep_uses_injectable_delay_without_waiting() {
    let error = anyhow::Error::new(LlmError::Overloaded {
        status: Some(529),
        retry_after_secs: None,
    });
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    for attempt in 1..PIPELINE_MAX_ATTEMPTS {
        let seen_by_sleeper = seen.clone();
        let delay = pipeline_attempt_retry_delay(attempt, &error);
        super::super::sleep_before_pipeline_retry(delay, move |delay| async move {
            seen_by_sleeper.lock().unwrap().push(delay);
        })
        .await;
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [Duration::from_secs(10), Duration::from_secs(20)]
    );
}

#[tokio::test]
async fn openai_compat_529_retry_after_reaches_pipeline_sleeper() {
    let error = anyhow::Error::new(LlmError::Server {
        status: 529,
        message: "overloaded".into(),
        pipeline_retry_after_secs: Some(120),
    });
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_by_sleeper = seen.clone();
    let delay = pipeline_attempt_retry_delay(1, &error);
    super::super::sleep_before_pipeline_retry(delay, move |delay| async move {
        seen_by_sleeper.lock().unwrap().push(delay);
    })
    .await;
    assert_eq!(*seen.lock().unwrap(), [Duration::from_secs(120)]);
}
