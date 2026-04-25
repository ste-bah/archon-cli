//! TASK-TUI-812 — Event latency p95 test.
//!
//! Implements NFR-TUI-PERF-005: P95 latency from producer `send()` to the
//! drain task observing the event must stay under 10ms at a sustained
//! 1000 events/sec rate.
//!
//! OQ-TUI-008 resolved the measurement-start question: the clock starts at
//! producer `send()` time (captured *before* `tx.send(...)`) and stops when
//! the drain task pulls the event off the receiver. That models the
//! send→render-apply path minus the render itself, which is what the
//! channel-latency NFR targets.
//!
//! The test is feature-gated behind `load-tests` so default
//! `cargo test -p archon-tui` remains fast; CI runs it via
//! `--features load-tests`.
//!
//! # Histogram unit trap (IMPORTANT)
//!
//! `archon_tui::observability::ChannelMetrics`'s internal histogram is
//! **ms-bucketed** with bounds `(1, 60_000, 3)`. Passing raw microseconds
//! would clip every value above 60 000 µs (60 ms) to the ceiling and poison
//! the exported p95. We therefore convert to whole milliseconds
//! (`elapsed.as_micros() / 1000`) before calling `record_latency_ms`.
//!
//! For the assertion itself we keep a **local** hdrhistogram with
//! microsecond bounds so p95 has sub-millisecond resolution — the shipped
//! ChannelMetrics histogram is too coarse to distinguish 5 ms from 9 ms.
//!
//! # Event variant choice
//!
//! Uses the zero-field `AgentEvent::UserPromptReady` variant (same as
//! TUI-810) so the measurement is dominated by channel + histogram
//! machinery, not by allocator traffic inside the event payload.

#![cfg(feature = "load-tests")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::AgentEvent;
use archon_tui::observability::ChannelMetrics;
use hdrhistogram::Histogram;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Tuning constants — pulled straight from NFR-TUI-PERF-005 / TASK-TUI-812 so
// regressions or tuning are one-line edits.
// ---------------------------------------------------------------------------

/// Target send rate: 1000 events/sec → one tick per 1000 µs.
const TICK_INTERVAL: Duration = Duration::from_micros(1000);
/// Run the producer for 5 seconds of wall-clock at 1000 ev/sec.
const RUN_DURATION: Duration = Duration::from_secs(5);
/// Total events the producer is expected to emit.
const TOTAL_EVENTS: usize = 5_000;
/// p95 budget in microseconds: 10 ms = 10 000 µs (NFR-TUI-PERF-005).
const P95_BUDGET_US: u64 = 10_000;
/// Local histogram microsecond bounds — 1 µs to 60 s, 3 sig figs.
const HIST_LO_US: u64 = 1;
const HIST_HI_US: u64 = 60_000_000;
const HIST_SIGFIG: u8 = 3;

// ---------------------------------------------------------------------------
// The latency test itself.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(not(feature = "load-tests"), ignore)]
async fn event_latency_p95() {
    // Channel payload carries (send_ts, event) so the drain side can compute
    // elapsed without needing a shared clock. Producer captures send_ts
    // immediately before calling tx.send (OQ-TUI-008).
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, AgentEvent)>();
    let metrics: Arc<ChannelMetrics> = Arc::new(ChannelMetrics::new());

    // --- Producer ----------------------------------------------------------
    let producer_metrics = metrics.clone();
    let producer_tx = tx.clone();
    let producer_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        // Skip missed ticks rather than bursting — we want steady 1 kHz,
        // not catch-up bursts that would mask latency regressions.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let start = Instant::now();
        let mut sent: usize = 0;
        // First tick fires immediately; run until wall-clock budget elapses.
        while start.elapsed() < RUN_DURATION {
            ticker.tick().await;
            // OQ-TUI-008: capture send_ts immediately before tx.send so the
            // measurement window starts at producer send() time.
            let send_ts = Instant::now();
            producer_tx
                .send((send_ts, AgentEvent::UserPromptReady))
                .expect("producer send failed — receiver dropped early");
            producer_metrics.record_sent();
            sent += 1;
        }
        sent
    });

    // --- Drain -------------------------------------------------------------
    // Use a local microsecond-bucketed hdrhistogram for p95 precision. The
    // shipped ChannelMetrics histogram is ms-bucketed (1, 60_000, 3) and
    // too coarse to distinguish single-digit-ms p95 values.
    let drain_metrics = metrics.clone();
    let drain_handle = tokio::spawn(async move {
        let mut local_hist: Histogram<u64> =
            Histogram::new_with_bounds(HIST_LO_US, HIST_HI_US, HIST_SIGFIG)
                .expect("hdrhistogram construction failed");
        while let Some((send_ts, _event)) = rx.recv().await {
            let elapsed = send_ts.elapsed();
            // Record into the local µs histogram for the p95 assertion.
            local_hist
                .record(elapsed.as_micros() as u64)
                .expect("hdrhistogram record out of range");
            // Cross-wire into the shipped metrics so the exporter also sees
            // these samples. Convert µs → ms to respect the ms-bucketed
            // histogram ceiling of 60_000 ms (passing raw µs would clip).
            drain_metrics.record_drained(1);
            drain_metrics.record_latency_ms((elapsed.as_micros() / 1000) as u64);
        }
        local_hist
    });

    // --- Shutdown ordering -------------------------------------------------
    // Wait for the producer to finish first.
    let sent_count = producer_handle.await.expect("producer task panicked");

    // Drop our own tx clone so the channel closes once the producer's clone
    // was dropped when its task ended. Without this the drain loop would
    // block forever on rx.recv().
    drop(tx);

    // Now the drain task will see None and return its local histogram.
    let local_hist = drain_handle.await.expect("drain task panicked");

    // --- Assertions --------------------------------------------------------

    // Sanity: we emitted roughly the expected number of events. Allow some
    // slack because interval drift on a loaded runner can cost a few ticks.
    // We require at least 90% of the target so a p95 computed over far
    // fewer samples can't sneak past the budget.
    assert!(
        sent_count >= (TOTAL_EVENTS * 9) / 10,
        "producer sent {} events, expected at least {} (90% of {})",
        sent_count,
        (TOTAL_EVENTS * 9) / 10,
        TOTAL_EVENTS,
    );

    let p50 = local_hist.value_at_quantile(0.50);
    let p95 = local_hist.value_at_quantile(0.95);
    let p99 = local_hist.value_at_quantile(0.99);

    // NFR-TUI-PERF-005: P95 < 10 ms (10 000 µs).
    assert!(
        p95 < P95_BUDGET_US,
        "p95 latency {} µs exceeds budget {} µs (NFR-TUI-PERF-005). \
         p50={} µs p99={} µs samples={}",
        p95,
        P95_BUDGET_US,
        p50,
        p99,
        local_hist.len(),
    );

    // Emit quantiles for CI log capture / humans reviewing runs.
    println!("p50={} p95={} p99={}", p50, p95, p99,);
    println!(
        "[event_latency_p95] samples={} sent={} p50_us={} p95_us={} p99_us={}",
        local_hist.len(),
        sent_count,
        p50,
        p95,
        p99,
    );
}
