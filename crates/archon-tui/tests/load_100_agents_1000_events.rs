//! TASK-TUI-810 — 100 agents × 1000 events sustained-throughput load test.
//!
//! Implements REQ-TUI-CHAN-003 (observability export) / AC-OBSERVABILITY-05 /
//! TC-TUI-OBSERVABILITY-06. The test is feature-gated behind `load-tests` so
//! default `cargo test -p archon-tui` remains fast; CI runs it via
//! `--features load-tests`.
//!
//! Assertions mirror TECH-TUI-OBSERVABILITY §LoadTestHarness (spec lines
//! 1174-1187):
//!   * every `tx.send(...)` returns `Ok` (producer never blocks).
//!   * `metrics.snapshot().total_sent == 100_000` exactly.
//!   * elapsed wall-clock < 10s (i.e. ≥10 000 events/sec sustained).
//!   * RSS delta ≤ 50 MiB per 1000 events (linear-growth bound).
//!
//! The test uses the cheapest AgentEvent variant (`UserPromptReady`, a
//! zero-field unit) so the work under test is dominated by the channel
//! machinery and the metrics counters, not by allocator traffic inside each
//! event payload. That matches the spec's intent: measure the channel, not
//! string allocation.

#![cfg(feature = "load-tests")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::AgentEvent;
use archon_tui::observability::ChannelMetrics;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::task::JoinSet;

mod common;
use common::memory::{assert_linear_memory_growth, rss_bytes};

// ---------------------------------------------------------------------------
// Tuning constants — pulled from the spec so regressions are one-line edits.
// ---------------------------------------------------------------------------

/// Number of concurrent producer tasks (REQ-TUI-CHAN-003).
const N_PRODUCERS: usize = 100;
/// Events per producer (REQ-TUI-CHAN-003).
const EVENTS_PER_PRODUCER: usize = 1_000;
/// Total events = 100 × 1000 = 100_000 (NFR-TUI-PERF-004 sustained rate).
const TOTAL_EVENTS: usize = N_PRODUCERS * EVENTS_PER_PRODUCER;
/// 100k events / 10k events-per-sec target = 10s wall-clock ceiling.
const ELAPSED_BUDGET: Duration = Duration::from_secs(10);
/// NFR-TUI-MEM-001: ≤50 MiB RSS delta per 1000 events.
const MAX_MB_PER_1K_EVENTS: f64 = 50.0;
/// Drain batch size — 256 matches the real session-stats drain loop so the
/// test exercises the same `record_drained` code path.
const DRAIN_BATCH: usize = 256;

// ---------------------------------------------------------------------------
// Helpers — kept in-file (test integration tests cannot share modules cleanly
// without a helper crate, and the harness is small enough to inline).
// ---------------------------------------------------------------------------

/// Spawn `n` producer tasks, each sending `events_per` `UserPromptReady`
/// events through `tx`. Every `tx.send(...)` must return `Ok`; a failed send
/// means the channel closed mid-flight, which is a hard test failure.
async fn spawn_n_producers(
    n: usize,
    events_per: usize,
    tx: UnboundedSender<AgentEvent>,
    metrics: Arc<ChannelMetrics>,
) -> JoinSet<()> {
    let mut set = JoinSet::new();
    for _producer_idx in 0..n {
        let tx = tx.clone();
        let metrics = metrics.clone();
        set.spawn(async move {
            for _ev in 0..events_per {
                // AC-OBSERVABILITY-05: producer must never block or fail.
                // The unbounded channel should return Ok until the receiver
                // is dropped, which only happens after producers finish.
                assert!(tx.send(AgentEvent::UserPromptReady).is_ok());
                metrics.record_sent();
            }
        });
    }
    set
}

// RSS + linear-growth helpers now live in `tests/common/memory.rs` (TASK-TUI-813).

// ---------------------------------------------------------------------------
// The load test itself.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_100_agents_1000_events() {
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let metrics: Arc<ChannelMetrics> = Arc::new(ChannelMetrics::new());

    // Capture starting RSS before any producers spawn so the delta we measure
    // bounds everything: channel buffers, metrics histograms, JoinSet tasks.
    let rss_start = rss_bytes();
    let start_ts = Instant::now();

    // Spawn drain task first so the channel never has an idle receiver once
    // producers start sending. Uses recv_many into a pre-sized Vec so each
    // metrics.record_drained() call sees a realistic batch size.
    let drain_metrics = metrics.clone();
    let drain_handle = tokio::spawn(async move {
        let mut buf: Vec<AgentEvent> = Vec::with_capacity(DRAIN_BATCH);
        let mut total_drained: u64 = 0;
        loop {
            let n = rx.recv_many(&mut buf, DRAIN_BATCH).await;
            if n == 0 {
                // All senders dropped — channel closed, drain is complete.
                break;
            }
            drain_metrics.record_drained(n as u64);
            total_drained += n as u64;
            buf.clear();
        }
        total_drained
    });

    // Spawn 100 producers × 1000 events and wait for all of them.
    let mut producer_set = spawn_n_producers(
        N_PRODUCERS,
        EVENTS_PER_PRODUCER,
        tx.clone(),
        metrics.clone(),
    )
    .await;

    // Drop our own tx so the channel closes once every producer finishes and
    // drops its clone. Without this the drain loop would block forever on
    // recv_many.
    drop(tx);

    while let Some(res) = producer_set.join_next().await {
        res.expect("producer task panicked");
    }

    // Now wait for the drain task to observe the channel close and finish.
    let total_drained = drain_handle.await.expect("drain task panicked");

    let elapsed = start_ts.elapsed();
    let end_ts = Instant::now();
    let rss_end = rss_bytes();
    let rss_delta = rss_end.saturating_sub(rss_start);
    // Two-point sample array for the shared linear-growth assertion. The
    // TASK-TUI-810 test only needs the endpoint compare; the 4-checkpoint
    // shape check lives in TASK-TUI-813's dedicated harness.
    let rss_samples: [(Instant, usize); 2] = [(start_ts, rss_start), (end_ts, rss_end)];

    // ── Assertions ─────────────────────────────────────────────────────────

    // 1) NFR-TUI-PERF-004: 100k events in under 10 seconds (≥10k ev/sec).
    assert!(
        elapsed < ELAPSED_BUDGET,
        "throughput regression: {:?} to drain {} events, budget {:?}",
        elapsed,
        TOTAL_EVENTS,
        ELAPSED_BUDGET,
    );

    // 2) total_sent counter sees every one of the 100k sends. Using the
    //    exact value (not `>=`) catches double-counting regressions as well
    //    as drops.
    let snap = metrics.snapshot();
    assert_eq!(
        snap.total_sent as usize, TOTAL_EVENTS,
        "total_sent mismatch: got {}, expected {}",
        snap.total_sent, TOTAL_EVENTS
    );

    // 3) Every sent event must also be drained — otherwise our record_drained
    //    path is buggy or the drain loop missed a batch.
    assert_eq!(
        total_drained as usize, TOTAL_EVENTS,
        "drain accounting mismatch: drained {}, expected {}",
        total_drained, TOTAL_EVENTS
    );

    // 4) NFR-TUI-MEM-001: memory stays within 50 MiB per 1000 events.
    assert_linear_memory_growth(&rss_samples, MAX_MB_PER_1K_EVENTS, TOTAL_EVENTS);

    eprintln!(
        "[load_100_agents_1000_events] elapsed={:?}, total_sent={}, \
         total_drained={}, rss_delta_bytes={}",
        elapsed, snap.total_sent, total_drained, rss_delta
    );
}
