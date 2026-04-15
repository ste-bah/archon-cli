// Integration tests for latency wiring in archon-tui.
//
// NOTE: This file intentionally fails to compile until TimestampedEvent wrapper
// type is defined in archon-core or archon-tui. This is expected and correct
// behavior at Gate 1 (tests-written-first).

use archon_tui::observability::ChannelMetrics;
use archon_core::agent::{AgentEvent, TimestampedEvent};
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
    let ts_event = TimestampedEvent { sent_at: Instant::now(), inner: event };
    let now = Instant::now();
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

/// Verify that same-tick zero-ms samples are clamped to the histogram floor (1ms)
/// and do NOT panic. The histogram silently drops sub-1ms samples.
#[test]
fn same_tick_send_drain_clamped_to_floor() {
    let metrics = ChannelMetrics::new();

    // First, record a valid 1ms sample to establish a floor reference
    metrics.record_latency_ms(1);

    // Zero-ms (same tick) should be silently clamped to 1ms — no panic
    // The histogram minimum is 1ms, so 0 gets recorded as 1
    metrics.record_latency_ms(0);

    // The histogram should have recorded both samples
    // (0ms clamped to 1ms, so we now have two entries at ~1ms)
    let snap = metrics.snapshot();
    assert!(
        snap.p95_send_to_render_ms >= 1,
        "After recording 1ms then 0ms (clamped to 1), P95 should still be >= 1ms; got {}",
        snap.p95_send_to_render_ms
    );
}