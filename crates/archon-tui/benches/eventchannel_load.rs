//! eventchannel_load bench — TUI-209
//! Verifies ChannelMetrics-instrumented unbounded channel sustains >=10k events/sec
//! across 100 producers × 1000 events each (100k total).
//!
//! AgentEvent variant: TextDelta("x") — smallest/cheapest variant, isolates channel throughput.

use criterion::{criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;

fn bench_eventchannel_load(_c: &mut Criterion) {
    let runtime = Runtime::new().expect("create tokio runtime");

    let result = runtime.block_on(async {
        let producers: usize = 100;
        let events_per_producer: usize = 1000;
        let total_events: usize = producers * events_per_producer;

        let metrics = Arc::new(archon_tui::observability::ChannelMetrics::new());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<archon_core::agent::TimestampedEvent>();

        // Producer tasks: each sends events_per_producer events
        let producer_handles: Vec<_> = (0..producers)
            .map(|_i| {
                let tx = tx.clone();
                let metrics = Arc::clone(&metrics);
                tokio::spawn(async move {
                    for _j in 0..events_per_producer {
                        // TextDelta("x") is the cheapest AgentEvent variant
                        let event = archon_core::agent::AgentEvent::TextDelta("x".to_string());
                        metrics.record_sent();
                        let timestamped = archon_core::agent::TimestampedEvent {
                            sent_at: Instant::now(),
                            inner: event,
                        };
                        let _ = tx.send(timestamped);
                    }
                })
            })
            .collect();

        // Single drain task: consume all events, record metrics
        let drain_metrics = Arc::clone(&metrics);
        let drain_handle = tokio::spawn(async move {
            let mut drained: usize = 0;
            while let Some(timestamped) = rx.recv().await {
                drained += 1;
                drain_metrics.record_drained(1);
                // Clamp latency to 1ms minimum per spec
                let latency_ms = 1.max(timestamped.sent_at.elapsed().as_millis() as u64);
                drain_metrics.record_latency_ms(latency_ms);
                if drained >= total_events {
                    break;
                }
            }
            drained
        });

        let start = Instant::now();

        // Wait for all producers to finish sending
        for handle in producer_handles {
            let _ = handle.await;
        }

        // Wait for drain to complete
        let drained_count = drain_handle.await.expect("drain task panicked");

        let duration = start.elapsed();
        let events_per_sec = total_events as f64 / duration.as_secs_f64();

        // Snapshot metrics before assertion
        let snapshot = metrics.snapshot();

        // Fixed workspace root for archonfixes worktree
        const WORKSPACE_ROOT: &str = "/home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes";
        let target_dir = std::path::Path::new(WORKSPACE_ROOT).join("target/tui-fixes");

        // Create directory and write JSON — BEFORE assertion so JSON persists on failure
        std::fs::create_dir_all(&target_dir).expect("create target/tui-fixes dir");
        let json_path = target_dir.join("eventchannel-load.json");
        let json = serde_json::json!({
            "task_id": "TASK-TUI-209",
            "producers": producers,
            "events_per_producer": events_per_producer,
            "total_events": total_events,
            "duration_secs": duration.as_secs_f64(),
            "events_per_sec": events_per_sec,
            "threshold_events_per_sec": 10000,
            "passed": events_per_sec >= 10_000.0,
            "channel_metrics": {
                "total_sent": snapshot.total_sent,
                "total_drained": snapshot.total_drained,
                "max_batch_size": snapshot.max_batch_size,
                "p95_send_to_render_ms": snapshot.p95_send_to_render_ms
            },
            "timestamp_utc": chrono::Utc::now().to_rfc3339()
        });
        let json_str = serde_json::to_string_pretty(&json).expect("serialize JSON");
        std::fs::write(&json_path, &json_str).expect("write JSON file");

        // Hard assertion AFTER JSON write so failure output is preserved
        assert!(
            events_per_sec >= 10_000.0,
            "Throughput {} events/sec below threshold 10000 events/sec (produced {} events in {:.3}s)",
            events_per_sec,
            drained_count,
            duration.as_secs_f64()
        );

        (events_per_sec, snapshot)
    });

    let (_events_per_sec, _snapshot) = result;
}

criterion_group!(benches, bench_eventchannel_load);
criterion_main!(benches);
