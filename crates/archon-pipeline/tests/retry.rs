//! Tests for TASK-PIPE-A05: API Retry with Exponential Backoff
//!
//! These tests verify:
//! - Success on first attempt (no retries)
//! - Retries on 429 (rate limit) errors
//! - Retries on 500, 502, 503 server errors
//! - Immediate failure on 400 (bad request)
//! - Immediate failure on 401, 403, 404, 422 (non-retryable)
//! - max_retries is honored (exactly max_retries+1 total attempts)
//! - retry_after hint is respected in delay calculation
//! - Exponential backoff delay progression
//! - Default config values

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Result;

use archon_pipeline::retry::{
    RetryConfig, RetryableError, calculate_delay, classify_error, with_retry,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates an operation closure that fails with a retryable 429 error for the
/// first `attempts_before_success` calls, then returns Ok("success").
fn make_failing_op(
    attempts_before_success: u32,
    counter: Arc<AtomicU32>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<String>>>> {
    move || {
        let counter = counter.clone();
        Box::pin(async move {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            if attempt < attempts_before_success {
                Err(anyhow::anyhow!(RetryableError::Retryable {
                    status: 429,
                    message: "Too Many Requests".into(),
                    retry_after: None,
                }))
            } else {
                Ok("success".to_string())
            }
        })
    }
}

/// Creates an operation closure that always fails with the given RetryableError.
fn make_always_failing_op(
    error: RetryableError,
    counter: Arc<AtomicU32>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<String>>>> {
    move || {
        let counter = counter.clone();
        let err = match &error {
            RetryableError::Retryable {
                status,
                message,
                retry_after,
            } => RetryableError::Retryable {
                status: *status,
                message: message.clone(),
                retry_after: *retry_after,
            },
            RetryableError::NonRetryable { status, message } => RetryableError::NonRetryable {
                status: *status,
                message: message.clone(),
            },
        };
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!(err))
        })
    }
}

/// Creates an operation that fails with a specific status code for the first N
/// calls, then succeeds. The status codes rotate through the provided list.
fn make_multi_status_failing_op(
    statuses: Vec<u16>,
    counter: Arc<AtomicU32>,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<String>>>> {
    let total_failures = statuses.len() as u32;
    move || {
        let counter = counter.clone();
        let statuses = statuses.clone();
        Box::pin(async move {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            if (attempt as usize) < statuses.len() {
                let status = statuses[attempt as usize];
                Err(anyhow::anyhow!(RetryableError::Retryable {
                    status,
                    message: format!("Server error {}", status),
                    retry_after: None,
                }))
            } else {
                Ok("success".to_string())
            }
        })
    }
}

/// No-jitter config with short delays for fast tests.
fn fast_config(max_retries: u32) -> RetryConfig {
    RetryConfig {
        max_retries,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_secs(60),
        jitter: false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. Operation succeeds immediately, no retries.
#[tokio::test]
async fn test_success_on_first_attempt() {
    let counter = Arc::new(AtomicU32::new(0));
    let config = fast_config(5);

    let c = counter.clone();
    let result = with_retry(&config, move || {
        let c = c.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok("immediate".to_string())
        }) as Pin<Box<dyn Future<Output = Result<String>>>>
    })
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "immediate");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "should call exactly once"
    );
}

/// 2. First 2 calls return 429, 3rd succeeds. Verify 3 total calls.
#[tokio::test]
async fn test_retries_on_429() {
    let counter = Arc::new(AtomicU32::new(0));
    let config = fast_config(5);

    let op = make_failing_op(2, counter.clone());
    let result = with_retry(&config, op).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "success");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "should attempt 3 times (2 failures + 1 success)"
    );
}

/// 3. Retries on 500, 502, 503 server errors.
#[tokio::test]
async fn test_retries_on_500_502_503() {
    let counter = Arc::new(AtomicU32::new(0));
    let config = fast_config(5);

    // Fail with 500, then 502, then 503, then succeed
    let op = make_multi_status_failing_op(vec![500, 502, 503], counter.clone());
    let result = with_retry(&config, op).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "success");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        4,
        "should attempt 4 times (3 failures + 1 success)"
    );
}

/// 4. 400 error is non-retryable, returns error after 1 attempt.
#[tokio::test]
async fn test_fails_immediately_on_400() {
    let counter = Arc::new(AtomicU32::new(0));
    let config = fast_config(5);

    let op = make_always_failing_op(
        RetryableError::NonRetryable {
            status: 400,
            message: "Bad Request".into(),
        },
        counter.clone(),
    );

    let result = with_retry(&config, op).await;

    assert!(result.is_err());
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "non-retryable error should stop after 1 attempt"
    );
}

/// 5. All non-retryable status codes fail immediately.
#[tokio::test]
async fn test_fails_immediately_on_401_403_404_422() {
    for status in [401u16, 403, 404, 422] {
        let counter = Arc::new(AtomicU32::new(0));
        let config = fast_config(5);

        let op = make_always_failing_op(
            RetryableError::NonRetryable {
                status,
                message: format!("Error {}", status),
            },
            counter.clone(),
        );

        let result = with_retry(&config, op).await;

        assert!(result.is_err(), "status {} should be non-retryable", status);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "status {} should stop after 1 attempt",
            status
        );
    }
}

/// 6. Operation always fails with 429. Verify exactly max_retries+1 total
///    attempts, then final error is returned.
#[tokio::test]
async fn test_max_retries_honored() {
    let max_retries = 3u32;
    let counter = Arc::new(AtomicU32::new(0));
    let config = fast_config(max_retries);

    let op = make_always_failing_op(
        RetryableError::Retryable {
            status: 429,
            message: "Too Many Requests".into(),
            retry_after: None,
        },
        counter.clone(),
    );

    let result = with_retry(&config, op).await;

    assert!(result.is_err(), "should fail after exhausting retries");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        max_retries + 1,
        "should attempt exactly max_retries+1 times (1 initial + {} retries)",
        max_retries
    );
}

/// 7. 429 with retry_after=5s. Verify the delay is at least the retry_after
///    value by checking calculate_delay with a retry_after hint.
#[tokio::test]
async fn test_retry_after_header_respected() {
    let counter = Arc::new(AtomicU32::new(0));

    // Use a config with short base_delay but we expect retry_after to dominate.
    let config = RetryConfig {
        max_retries: 3,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_secs(60),
        jitter: false,
    };

    let retry_after = Duration::from_millis(50);

    // Create an op that returns 429 with retry_after on the first call,
    // then succeeds.
    let c = counter.clone();
    let op = move || {
        let c = c.clone();
        Box::pin(async move {
            let attempt = c.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                Err(anyhow::anyhow!(RetryableError::Retryable {
                    status: 429,
                    message: "Too Many Requests".into(),
                    retry_after: Some(Duration::from_millis(50)),
                }))
            } else {
                Ok("success".to_string())
            }
        }) as Pin<Box<dyn Future<Output = Result<String>>>>
    };

    let start = std::time::Instant::now();
    let result = with_retry(&config, op).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    // The delay should be at least the retry_after duration (50ms).
    // We use a slightly lower threshold to account for timing precision.
    assert!(
        elapsed >= Duration::from_millis(40),
        "elapsed {:?} should be at least ~50ms (retry_after)",
        elapsed
    );
}

/// 8. Verify calculate_delay produces exponential growth:
///    base*1, base*2, base*4, base*8, capped at max_delay.
#[tokio::test]
async fn test_exponential_backoff_delays() {
    let config = RetryConfig {
        max_retries: 10,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(1000),
        jitter: false,
    };

    // attempt 0: 100ms * 2^0 = 100ms
    assert_eq!(calculate_delay(&config, 0), Duration::from_millis(100));

    // attempt 1: 100ms * 2^1 = 200ms
    assert_eq!(calculate_delay(&config, 1), Duration::from_millis(200));

    // attempt 2: 100ms * 2^2 = 400ms
    assert_eq!(calculate_delay(&config, 2), Duration::from_millis(400));

    // attempt 3: 100ms * 2^3 = 800ms
    assert_eq!(calculate_delay(&config, 3), Duration::from_millis(800));

    // attempt 4: 100ms * 2^4 = 1600ms -> capped at 1000ms
    assert_eq!(calculate_delay(&config, 4), Duration::from_millis(1000));

    // attempt 5: still capped at 1000ms
    assert_eq!(calculate_delay(&config, 5), Duration::from_millis(1000));
}

/// 9. Verify RetryConfig::default() has correct values.
#[tokio::test]
async fn test_default_config() {
    let config = RetryConfig::default();

    assert_eq!(config.max_retries, 5);
    assert_eq!(config.base_delay, Duration::from_secs(1));
    assert_eq!(config.max_delay, Duration::from_secs(60));
    assert!(config.jitter, "jitter should be enabled by default");
}
