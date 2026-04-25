//! Integration test for warn_if_backlog_over rate-limit behaviour.
//!
//! Tests the three properties required by spec:
//! 1. Suppressed below threshold (no warn when backlog <= threshold)
//! 2. Rate-limited: at most one warn per 1000ms while over threshold
//! 3. Monotonic: last_warn_unix_ms only increases, never decreases

use archon_tui::observability::ChannelMetrics;
use std::time::Duration;

// --- Test 1: warn suppressed below threshold ---

#[tokio::test]
async fn warn_suppressed_below_threshold() {
    let metrics = ChannelMetrics::new();

    // Record 9_999 sends — backlog is 9_999, threshold is 10_000
    for _ in 0..9_999 {
        metrics.record_sent();
    }

    // warn_if_backlog_over should return false (not over threshold)
    let fired = metrics.warn_if_backlog_over(10_000);
    assert!(
        !fired,
        "warn should NOT fire when backlog={} <= threshold=10000",
        metrics
            .backlog_depth
            .load(std::sync::atomic::Ordering::Relaxed)
    );
}

// --- Test 2: rate-limit fires once per second ---

#[test]
fn warn_fires_once_per_second_over_threshold() {
    let metrics = ChannelMetrics::new();

    // Record 15_000 sends — backlog well over 10_000 threshold
    for _ in 0..15_000 {
        metrics.record_sent();
    }

    // First call should fire (not rate-limited yet, last_warn_unix_ms == 0)
    let first = metrics.warn_if_backlog_over(10_000);
    assert!(first, "first call should fire when over threshold");

    // Second and third calls in rapid succession should be suppressed
    let second = metrics.warn_if_backlog_over(10_000);
    let third = metrics.warn_if_backlog_over(10_000);
    assert!(
        !second,
        "second call should be rate-limited (within 1s window)"
    );
    assert!(
        !third,
        "third call should be rate-limited (within 1s window)"
    );

    // Drain all events so we're no longer over threshold
    for _ in 0..15_000 {
        metrics.record_drained(1);
    }

    // Wait for rate window to expire using blocking sleep (wall-clock guaranteed)
    std::thread::sleep(Duration::from_millis(1_100));

    // Refill backlog over threshold
    for _ in 0..15_000 {
        metrics.record_sent();
    }

    // After rate window, should fire again
    let after_sleep = metrics.warn_if_backlog_over(10_000);
    assert!(
        after_sleep,
        "after 1.1s sleep, warn should fire again (backlog={})",
        metrics
            .backlog_depth
            .load(std::sync::atomic::Ordering::Relaxed)
    );
}

// --- Test 3: rate-limit is monotonic ---

#[test]
fn warn_rate_limit_is_monotonic() {
    let metrics = ChannelMetrics::new();

    // last_warn_unix_ms starts at 0
    let initial = metrics
        .last_warn_unix_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(initial, 0, "last_warn_unix_ms should initialize to 0");

    // Fill over threshold and fire
    for _ in 0..15_000 {
        metrics.record_sent();
    }
    let first_fire = metrics.warn_if_backlog_over(10_000);
    assert!(first_fire, "first fire should succeed");
    let first_ts = metrics
        .last_warn_unix_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        first_ts > 0,
        "last_warn_unix_ms should be set after first fire"
    );

    // Drain and wait
    for _ in 0..15_000 {
        metrics.record_drained(1);
    }
    std::thread::sleep(Duration::from_millis(1_100));

    // Fire again — timestamp should increase
    for _ in 0..15_000 {
        metrics.record_sent();
    }
    let second_fire = metrics.warn_if_backlog_over(10_000);
    assert!(second_fire, "second fire after 1.1s should succeed");
    let second_ts = metrics
        .last_warn_unix_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        second_ts >= first_ts,
        "last_warn_unix_ms must be monotonic: {} >= {}",
        second_ts,
        first_ts
    );
}
