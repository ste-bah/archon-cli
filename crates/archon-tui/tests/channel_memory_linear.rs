//! TASK-TUI-813 — dedicated linear-growth RSS sampling test.
//!
//! Implements REQ-TUI-CHAN-004 [8/8: observability gate], AC-OBSERVABILITY-06,
//! TC-TUI-OBSERVABILITY-07. Complements TASK-TUI-810's end-vs-start RSS
//! assertion by sampling RSS at four checkpoints during a sustained
//! 10 000-event run and asserting:
//!   * end-to-end ratio stays within 50 MiB per 1000 events
//!     (NFR-TUI-MEM-001);
//!   * consecutive-checkpoint slopes stay within 2× the mean
//!     (catches super-linear leaks the endpoint compare would miss);
//!   * `ChannelMetrics::snapshot().backlog_depth == 0` at end
//!     (proves drain fully caught up).
//!
//! Feature-gated behind `load-tests` like the other TUI load tests.

#![cfg(feature = "load-tests")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::AgentEvent;
use archon_tui::observability::ChannelMetrics;
use tokio::sync::mpsc;

mod common;
use common::memory::{assert_linear_memory_growth_at, rss_bytes};

/// Total sustained load — the harness produces this many events and drains
/// to zero backlog between checkpoints so RSS samples reflect only retained
/// memory, not transient in-flight buffers.
const TOTAL_EVENTS: usize = 10_000;
/// Checkpoints (cumulative event counts). Matches spec TC-TUI-OBSERVABILITY-07.
const CHECKPOINTS: [usize; 4] = [1_000, 2_000, 5_000, 10_000];
/// NFR-TUI-MEM-001: ≤50 MiB RSS delta per 1000 events.
const MAX_MB_PER_1K_EVENTS: f64 = 50.0;
/// Drain batch size — matches the real session-stats drain loop.
const DRAIN_BATCH: usize = 256;

/// Pump `count` events through `tx` (each as a zero-field `UserPromptReady`),
/// recording each send on `metrics`.
fn produce(count: usize, tx: &mpsc::UnboundedSender<AgentEvent>, metrics: &Arc<ChannelMetrics>) {
    for _ in 0..count {
        assert!(tx.send(AgentEvent::UserPromptReady).is_ok());
        metrics.record_sent();
    }
}

/// Wait for `rx` to drain fully. Returns once `backlog_depth == 0` (or
/// panics after `timeout`). Relies on the drain task running concurrently
/// to pull events out of the channel.
async fn wait_for_drain(metrics: &Arc<ChannelMetrics>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if metrics.snapshot().backlog_depth == 0 {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "drain did not complete within {:?}; backlog_depth={}",
                timeout,
                metrics.snapshot().backlog_depth
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[cfg_attr(not(feature = "load-tests"), ignore)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channel_memory_linear_growth_under_sustained_load() {
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let metrics: Arc<ChannelMetrics> = Arc::new(ChannelMetrics::new());

    // Drain task — runs for the whole test, record_drained on each batch.
    let drain_metrics = metrics.clone();
    let drain_handle = tokio::spawn(async move {
        let mut buf: Vec<AgentEvent> = Vec::with_capacity(DRAIN_BATCH);
        let mut total_drained: u64 = 0;
        loop {
            let n = rx.recv_many(&mut buf, DRAIN_BATCH).await;
            if n == 0 {
                break;
            }
            drain_metrics.record_drained(n as u64);
            total_drained += n as u64;
            buf.clear();
        }
        total_drained
    });

    // Baseline sample before any events — index 0 of `samples`.
    let mut samples: Vec<(Instant, usize)> = Vec::with_capacity(CHECKPOINTS.len() + 1);
    samples.push((Instant::now(), rss_bytes()));
    eprintln!(
        "[channel_memory_linear] baseline rss={} bytes ({:.2} MiB)",
        samples[0].1,
        samples[0].1 as f64 / 1_048_576.0,
    );

    let mut produced: usize = 0;
    for &target in CHECKPOINTS.iter() {
        let delta = target - produced;
        produce(delta, &tx, &metrics);
        produced = target;

        // Wait for drain to catch up so RSS is measured at steady state.
        wait_for_drain(&metrics, Duration::from_secs(5)).await;

        let sample = (Instant::now(), rss_bytes());
        let prev_rss = samples.last().unwrap().1;
        let delta_bytes = sample.1 as i64 - prev_rss as i64;
        eprintln!(
            "[channel_memory_linear] checkpoint events={} rss={} bytes \
             ({:.2} MiB) delta_vs_prev={} bytes",
            target,
            sample.1,
            sample.1 as f64 / 1_048_576.0,
            delta_bytes,
        );
        samples.push(sample);
    }

    // Close channel so drain task exits.
    drop(tx);
    let total_drained = drain_handle.await.expect("drain task panicked");

    // ── Assertions ─────────────────────────────────────────────────────────

    // 1) Every sent event was also drained.
    let snap = metrics.snapshot();
    assert_eq!(
        snap.total_sent as usize, TOTAL_EVENTS,
        "total_sent mismatch: got {}, expected {}",
        snap.total_sent, TOTAL_EVENTS,
    );
    assert_eq!(
        total_drained as usize, TOTAL_EVENTS,
        "drain accounting mismatch: drained {}, expected {}",
        total_drained, TOTAL_EVENTS,
    );

    // 2) Backlog at end must be zero (proves drain fully caught up).
    assert_eq!(
        snap.backlog_depth, 0,
        "backlog_depth must be 0 at end, got {}",
        snap.backlog_depth,
    );

    // 3) Linear-growth bound — sample-based shape check. Feed the ACTUAL
    //    checkpoint event counts (0 for baseline, then 1k/2k/5k/10k) so the
    //    slope analysis reflects real load rather than the equal-spacing
    //    default in the legacy helper.
    let mut event_counts: Vec<usize> = Vec::with_capacity(CHECKPOINTS.len() + 1);
    event_counts.push(0);
    event_counts.extend_from_slice(&CHECKPOINTS);
    assert_linear_memory_growth_at(&samples, &event_counts, MAX_MB_PER_1K_EVENTS);

    eprintln!(
        "[channel_memory_linear] PASS total_sent={} total_drained={} \
         backlog_depth={} samples={:?}",
        snap.total_sent, total_drained, snap.backlog_depth, samples,
    );
}
