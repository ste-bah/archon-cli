// Integration tests for latency wiring in archon-tui.
//
// (The header used to say this file intentionally failed to compile until
// `TimestampedEvent` existed. It has existed for a long time; the note was
// stale.)

use archon_core::agent::{AgentEvent, TimestampedEvent};
use archon_tui::observability::ChannelMetrics;
use std::time::Instant;

/// The producer stamps `sent_at` at the moment of send, not earlier.
///
/// This used to build a `TimestampedEvent` inside the test body with
/// `Instant::now()` and then assert that `Instant::now().elapsed() <= 10ms`. It
/// never touched the producer, so it could not fail for any reason connected to
/// the code it claims to cover — only if the machine stalled for 10ms between
/// two adjacent statements, which is a flake, not a finding. It measured the
/// clock and reported it as latency wiring.
///
/// `Agent::send_event` is `pub(super)` inside `archon-core` and needs a live LLM
/// client to reach, so it cannot be driven from this crate. What is checkable
/// here is the producer's source: the stamp must be taken *in* the send, from
/// `Instant::now()`, rather than carried in from a struct built earlier — a
/// stale or defaulted `Instant` would leave every latency reading wrong while
/// the field stayed dutifully populated. `tests/tc_arch_05_grep_agent_event_send.rs`
/// guards the neighbouring line of the same function the same way.
#[test]
fn sent_at_is_stamped_by_the_producer_at_send_time() {
    let events_rs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../archon-core/src/agent/events.rs");
    let source = std::fs::read_to_string(&events_rs)
        .unwrap_or_else(|e| panic!("read {}: {e}", events_rs.display()));

    let send_event = source
        .split_once("async fn send_event(")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| {
            panic!(
                "no `send_event` in {} — the agent event producer has moved, and this \
                 check is inspecting nothing",
                events_rs.display()
            )
        });

    assert!(
        send_event.contains("sent_at: std::time::Instant::now()"),
        "`send_event` must stamp `sent_at` from `Instant::now()` at the point of \
         send. Without it, every send-to-render latency figure the TUI reports is \
         measured from the wrong instant."
    );

    // The type still has to carry the field the producer stamps; a rename here
    // would otherwise leave the source check above matching dead prose.
    let ts = TimestampedEvent {
        sent_at: Instant::now(),
        inner: AgentEvent::TextDelta("test".to_string()),
    };
    assert!(matches!(ts.inner, AgentEvent::TextDelta(_)));
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
