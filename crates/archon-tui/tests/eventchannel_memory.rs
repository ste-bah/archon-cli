//! eventchannel_memory — TUI-210
//! Integration test proving channel memory growth stays under 50KB/event (peak-baseline)
//! with a 50MB ceiling across stepped bursts (1k, 2k, 4k, 8k events).
//!
//! AgentEvent variant: TextDelta("x") — same cheapest/cheapest variant as TUI-209.
//! Using inline synthetic generator (Path B) per spec — no shared helper module.

// The whole test is fundamentally Linux-specific: it reads VmRSS from
// /proc/self/status via the `procfs` crate. On macOS/Windows there is no
// /proc and procfs is not buildable, so the file is gated end-to-end with
// `#[cfg(target_os = "linux")]`. The Cargo.toml gate on the `procfs`
// dev-dep mirrors this. (TASK-CI-PORTABILITY-HOTFIX / closes #220.)
#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Instant;

fn read_rss_bytes() -> u64 {
    // VmRSS in /proc/self/status is reported in kB — multiply by 1024 to get bytes
    let me = procfs::process::Process::myself().expect("procfs self");
    let stat = me.status().expect("procfs status");
    stat.vmrss.expect("VmRSS field") * 1024
}

#[tokio::test(flavor = "current_thread")]
async fn eventchannel_memory_growth_test() {
    /*
     * WARM-UP RATIONALE:
     * Tokio runtime + mpsc channel + procfs crate all allocate on first use.
     * Without warm-up, baseline RSS captures only partial initialization cost,
     * making first-burst delta artificially large. 10 small send+drain cycles
     * before baseline sample stabilize the allocator.
     */
    const WARMUP_CYCLES: usize = 10;
    const BURST_SIZES: [usize; 4] = [1000, 2000, 4000, 8000];
    const THRESHOLD_BYTES_PER_EVENT: f64 = 50_000.0;
    const CEILING_BYTES_TOTAL: u64 = 50 * 1024 * 1024;

    // Warm-up phase: 10 send+drain cycles to stabilize allocator slack
    for cycle in 0..WARMUP_CYCLES {
        let (tx, rx) =
            tokio::sync::mpsc::unbounded_channel::<archon_core::agent::TimestampedEvent>();
        let metrics = Arc::new(archon_tui::observability::ChannelMetrics::new());

        // Spawn producer and drain tasks concurrently
        let metrics_clone = Arc::clone(&metrics);
        let producer = tokio::spawn(async move {
            for i in 0..100 {
                // TextDelta("x") — same variant as TUI-209 (line 31 in eventchannel_load.rs)
                let event =
                    archon_core::agent::AgentEvent::TextDelta(format!("warmup-{}-{}", cycle, i));
                metrics_clone.record_sent();
                let timestamped = archon_core::agent::TimestampedEvent {
                    sent_at: Instant::now(),
                    inner: event,
                };
                let _ = tx.send(timestamped);
            }
        });

        let drain_metrics = Arc::clone(&metrics);
        let consumer = tokio::spawn(async move {
            let mut drained: usize = 0;
            let mut rx = rx;
            while let Some(timestamped) = rx.recv().await {
                drained += 1;
                drain_metrics.record_drained(1);
                let latency_ms = 1.max(timestamped.sent_at.elapsed().as_millis() as u64);
                drain_metrics.record_latency_ms(latency_ms);
            }
            drained
        });

        producer.await.expect("producer panicked");
        let _drained = consumer.await.expect("consumer panicked");
    }

    // Baseline RSS sample: AFTER runtime init and warm-up, BEFORE first send
    let baseline_rss = read_rss_bytes();

    let mut bursts_results = Vec::new();
    let mut max_bytes_per_event_observed: f64 = 0.0;
    let mut max_delta_bytes_observed: u64 = 0;
    let mut final_total_sent: u64 = 0;
    let mut final_total_drained: u64 = 0;
    let mut final_max_batch_size: u64 = 0;
    let mut final_p95_send_to_render_ms: u64 = 0;

    for &burst_size in &BURST_SIZES {
        // Create fresh channel for this burst
        let (tx, rx) =
            tokio::sync::mpsc::unbounded_channel::<archon_core::agent::TimestampedEvent>();
        let metrics = Arc::new(archon_tui::observability::ChannelMetrics::new());

        // Spawn producer and consumer concurrently
        let metrics_clone = Arc::clone(&metrics);
        let producer = tokio::spawn(async move {
            for i in 0..burst_size {
                // TextDelta("x") — cheapest AgentEvent variant, isolates channel throughput
                let event = archon_core::agent::AgentEvent::TextDelta(format!("event-{}", i));
                metrics_clone.record_sent();
                let timestamped = archon_core::agent::TimestampedEvent {
                    sent_at: Instant::now(),
                    inner: event,
                };
                let _ = tx.send(timestamped);
            }
        });

        let drain_metrics = Arc::clone(&metrics);
        let consumer = tokio::spawn(async move {
            let mut drained: usize = 0;
            let mut rx = rx;
            while let Some(timestamped) = rx.recv().await {
                drained += 1;
                drain_metrics.record_drained(1);
                // Clamp latency to 1ms minimum per spec
                let latency_ms = 1.max(timestamped.sent_at.elapsed().as_millis() as u64);
                drain_metrics.record_latency_ms(latency_ms);
            }
            drained
        });

        producer.await.expect("producer panicked");
        let drained_count = consumer.await.expect("consumer panicked");
        assert_eq!(
            drained_count, burst_size,
            "drain count mismatch for burst {}",
            burst_size
        );

        // Sample peak RSS IMMEDIATELY AFTER drain
        let peak_rss = read_rss_bytes();
        let delta_bytes = peak_rss.saturating_sub(baseline_rss);
        let bytes_per_event = delta_bytes as f64 / burst_size as f64;

        max_bytes_per_event_observed = max_bytes_per_event_observed.max(bytes_per_event);
        max_delta_bytes_observed = max_delta_bytes_observed.max(delta_bytes);

        // Capture metrics snapshot
        let snapshot = metrics.snapshot();
        final_total_sent = snapshot.total_sent;
        final_total_drained = snapshot.total_drained;
        final_max_batch_size = snapshot.max_batch_size;
        final_p95_send_to_render_ms = snapshot.p95_send_to_render_ms;

        bursts_results.push(serde_json::json!({
            "events": burst_size,
            "baseline_rss_bytes": baseline_rss,
            "peak_rss_bytes": peak_rss,
            "delta_bytes": delta_bytes,
            "bytes_per_event": bytes_per_event
        }));
    }

    // Build JSON — MUST happen BEFORE assertions so artifact persists on failure
    let json = serde_json::json!({
        "task_id": "TASK-TUI-210",
        "bursts": bursts_results,
        "threshold_bytes_per_event": THRESHOLD_BYTES_PER_EVENT,
        "ceiling_bytes_total": CEILING_BYTES_TOTAL,
        "max_bytes_per_event_observed": max_bytes_per_event_observed,
        "max_delta_bytes_observed": max_delta_bytes_observed,
        "passed": max_bytes_per_event_observed < THRESHOLD_BYTES_PER_EVENT
            && max_delta_bytes_observed < CEILING_BYTES_TOTAL,
        "channel_metrics_final": {
            "total_sent": final_total_sent,
            "total_drained": final_total_drained,
            "max_batch_size": final_max_batch_size,
            "p95_send_to_render_ms": final_p95_send_to_render_ms
        },
        "environment": "wsl2",
        "timestamp_utc": chrono::Utc::now().to_rfc3339()
    });

    // Write JSON artifact to a per-test tempdir. Previously hardcoded to a
    // developer-local absolute path which broke under cargo-llvm-cov's
    // sandbox and on any CI runner. (TASK-CI-PORTABILITY-HOTFIX / closes
    // #222.)
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let json_path = tmp.path().join("eventchannel-memory.json");
    let json_str = serde_json::to_string_pretty(&json).expect("serialize JSON");
    std::fs::write(&json_path, &json_str).expect("write JSON file");

    // Assertions AFTER JSON write so artifact persists on failure
    assert!(
        max_bytes_per_event_observed < THRESHOLD_BYTES_PER_EVENT,
        "bytes_per_event {} exceeds threshold {} (baseline={}, max_delta={})",
        max_bytes_per_event_observed,
        THRESHOLD_BYTES_PER_EVENT,
        baseline_rss,
        max_delta_bytes_observed
    );

    assert!(
        max_delta_bytes_observed < CEILING_BYTES_TOTAL,
        "delta_bytes {} exceeds ceiling {} (baseline={}, bytes_per_event={})",
        max_delta_bytes_observed,
        CEILING_BYTES_TOTAL,
        baseline_rss,
        max_bytes_per_event_observed
    );
}
