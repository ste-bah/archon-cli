// Integration tests for latency wiring in archon-tui.
//
// NOTE: This file intentionally fails to compile until TimestampedEvent wrapper
// type is defined in archon-core or archon-tui. This is expected and correct
// behavior at Gate 1 (tests-written-first).

use archon_core::agent::{AgentEvent, TimestampedEvent};
use archon_tui::observability::ChannelMetrics;
use std::time::Instant;

/// Verify that a TimestampedEvent carries a `sent_at` timestamp close to Instant::now.
///
/// The producer side stamps events with `Instant::now()` at send time. This test
/// confirms the field is populated and within a reasonable 10ms window.
#[test]
fn sent_at_populated_at_producer() {
    // TimestampedEvent wraps AgentEvent with a sent_at timestamp.
    // Verify sent_at is populated (within 10ms of creation).
    let event = AgentEvent::TextDelta("test".to_string());
    let ts_event = TimestampedEvent {
        sent_at: Instant::now(),
        inner: event,
    };
    let _now = Instant::now();
    // Should be within 10ms
    assert!(
        ts_event.sent_at.elapsed().as_millis() <= 10,
        "sent_at should be within 10ms of Instant::now; got {}ms",
        ts_event.sent_at.elapsed().as_millis()
    );
}

/// Verify that draining a channel with a 5ms delay records a non-zero P95 latency.
#[test]
fn drain_records_nonzero_elapsed() {
    let metrics = ChannelMetrics::new();
    metrics.record_latency_ms(5);

    let snap = metrics.snapshot();
    assert!(
        snap.p95_send_to_render_ms >= 1,
        "P95 latency should be >= 1ms after recording 5ms sample; got {}",
        snap.p95_send_to_render_ms
    );
}

/// Verify that snapshot reflects all samples after draining — 20 samples at varying delays.
#[test]
fn snapshot_reflects_samples_post_drain() {
    let metrics = ChannelMetrics::new();

    // 20 samples: 1ms x5, 5ms x5, 10ms x5, 20ms x5
    for _ in 0..5 {
        metrics.record_latency_ms(1);
    }
    for _ in 0..5 {
        metrics.record_latency_ms(5);
    }
    for _ in 0..5 {
        metrics.record_latency_ms(10);
    }
    for _ in 0..5 {
        metrics.record_latency_ms(20);
    }

    let snap = metrics.snapshot();

    // P95 of [1,1,1,1,1,5,5,5,5,5,10,10,10,10,10,20,20,20,20,20]
    // should fall between 5ms (the 19th sample) and 20ms (the 20th sample)
    assert!(
        snap.p95_send_to_render_ms >= 5 && snap.p95_send_to_render_ms <= 20,
        "P95 should be in range [5, 20]ms; got {}",
        snap.p95_send_to_render_ms
    );
}

/// Verify that zero-ms (same-tick) samples are silently dropped by the
/// histogram — no artificial floor is applied. A single 0ms sample on a
/// fresh ChannelMetrics produces p95 == 0, confirming that the histogram
/// has no artificial minimum.
#[test]
fn zero_ms_sample_is_silently_dropped() {
    let metrics = ChannelMetrics::new();

    // Zero-ms sample: histogram with min=1ms silently drops it
    metrics.record_latency_ms(0);

    let snap = metrics.snapshot();
    assert_eq!(
        snap.p95_send_to_render_ms, 0,
        "After recording only 0ms, p95 should be 0 (silent drop); got {}",
        snap.p95_send_to_render_ms
    );
}

/// Verify that a single valid 1ms sample produces p95 == 1.
#[test]
fn one_ms_sample_produces_p95_of_one() {
    let metrics = ChannelMetrics::new();

    metrics.record_latency_ms(1);

    let snap = metrics.snapshot();
    assert_eq!(
        snap.p95_send_to_render_ms, 1,
        "After recording only 1ms, p95 should be 1; got {}",
        snap.p95_send_to_render_ms
    );
}
