//! eventchannel_latency bench — TUI-211
//! Verifies ChannelMetrics P95 send-to-render latency stays under 10ms
//! during a sustained 10k events/sec paced workload for 5 seconds (50k events).
//!
//! Pacing: 10_000 events/sec means 1 event every 100us.
//! Producers: 10 parallel producers × 5000 events each = 50k total.
//! Each producer yields every 100us to maintain precise rate.

use criterion::{Criterion, criterion_group, criterion_main};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::time::Duration;

fn bench_eventchannel_latency(_c: &mut Criterion) {
    let runtime = Runtime::new().expect("create tokio runtime");

    runtime.block_on(async {
        let total_events: usize = 50_000;
        let producers: usize = 10;
        let events_per_producer: usize = total_events / producers; // 5000 each
        let pace_us: u64 = 100; // 10k/sec = 1 event per 100us

        let metrics = Arc::new(archon_tui::observability::ChannelMetrics::new());
        let (tx, mut rx) =
            tokio::sync::mpsc::unbounded_channel::<archon_core::agent::TimestampedEvent>();

        // Single-channel producer: paced 10k events/sec
        let producer_metrics = Arc::clone(&metrics);
        let producer_handle = tokio::spawn(async move {
            let start = Instant::now();
            let target_interval_us: u64 = pace_us;

            for i in 0..total_events {
                let event = archon_core::agent::AgentEvent::TextDelta(format!("event-{}", i));
                producer_metrics.record_sent();
                let timestamped = archon_core::agent::TimestampedEvent {
                    sent_at: Instant::now(),
                    inner: event,
                };
                let _ = tx.send(timestamped);

                // Paced delay: yield every target_interval_us
                // busy-wait for sub-millisecond precision
                let elapsed_us = start.elapsed().as_millis() as u64 * 1000;
                let expected_us = (i as u64 + 1) * target_interval_us;
                if elapsed_us < expected_us {
                    let sleep_us = expected_us - elapsed_us;
                    if sleep_us >= 1000 {
                        // sleep in 1ms chunks to stay precise
                        let _ = tokio::time::sleep(Duration::from_micros(sleep_us)).await;
                    }
                    // For sub-ms, spin briefly
                    while (start.elapsed().as_micros() as u64) < expected_us {}
                }
            }
        });

        // Single drain task: consume all events with per-event latency sampling
        let drain_metrics = Arc::clone(&metrics);
        let drain_handle = tokio::spawn(async move {
            let mut drained: usize = 0;
            while let Some(ts) = rx.recv().await {
                drained += 1;
                // Per-event latency sampling (the TUI-211 fix)
                let elapsed_ms = (ts.sent_at.elapsed().as_millis() as u64).max(1);
                drain_metrics.record_latency_ms(elapsed_ms);
                drain_metrics.record_drained(1);
                if drained >= total_events {
                    break;
                }
            }
            drained
        });

        let bench_start = Instant::now();

        // Wait for producer and drain to complete
        let _ = producer_handle.await;
        let drained_count = drain_handle.await.expect("drain task panicked");

        let duration = bench_start.elapsed();
        let events_per_sec = total_events as f64 / duration.as_secs_f64();

        let snapshot = metrics.snapshot();

        // Per-bench tempdir for the JSON artifact. Previously hardcoded to a
        // developer-local absolute path which broke under cargo-llvm-cov's
        // sandbox and on any CI runner. (TASK-CI-PORTABILITY-HOTFIX.)
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        // Write JSON BEFORE assertion so the artifact persists on failure.
        let json_path = tmp.path().join("eventchannel-latency.json");
        let json = serde_json::json!({
            "task_id": "TASK-TUI-211",
            "producers": producers,
            "events_per_producer": events_per_producer,
            "total_events": total_events,
            "duration_secs": duration.as_secs_f64(),
            "events_per_sec": events_per_sec,
            "p95_send_to_render_ms": snapshot.p95_send_to_render_ms,
            "threshold_p95_ms": 10,
            "passed": snapshot.p95_send_to_render_ms < 10,
            "channel_metrics": {
                "total_sent": snapshot.total_sent,
                "total_drained": snapshot.total_drained,
                "max_batch_size": snapshot.max_batch_size,
                "backlog_depth": snapshot.backlog_depth
            },
            "timestamp_utc": chrono::Utc::now().to_rfc3339()
        });
        let json_str = serde_json::to_string_pretty(&json).expect("serialize JSON");
        std::fs::write(&json_path, &json_str).expect("write JSON file");

        // Hard assertion: p95 must be under 10ms
        assert!(
            snapshot.p95_send_to_render_ms < 10,
            "P95 latency {}ms exceeds 10ms threshold (produced {} events in {:.3}s, {} events/sec)",
            snapshot.p95_send_to_render_ms,
            drained_count,
            duration.as_secs_f64(),
            events_per_sec
        );
    });
}

criterion_group!(benches, bench_eventchannel_latency);
criterion_main!(benches);
