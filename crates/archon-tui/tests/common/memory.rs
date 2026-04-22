//! TASK-TUI-813: Shared RSS sampling + linear-growth assertion helpers.
//!
//! References:
//!   - 02-technical-spec.md TECH-TUI-OBSERVABILITY lines 1174-1179
//!     (`assert_linear_memory_growth` harness helper signature).
//!   - 01-functional-spec.md REQ-TUI-CHAN-004 line 1159,
//!     AC-OBSERVABILITY-06 line 1140, TC-TUI-OBSERVABILITY-07 line 1197.
//!
//! Consumed by:
//!   - `tests/load_100_agents_1000_events.rs` (TASK-TUI-810 refactor)
//!   - `tests/channel_memory_linear.rs` (TASK-TUI-813)
//!
//! Kept minimal on purpose: no libc/jemalloc dependency. Linux reads
//! `/proc/self/statm`; other OS fall back to 0 and the caller treats the
//! samples as unavailable (no false-negative panics on macOS/WSL corner
//! cases). The assertion surface matches the spec's `LoadTestHarness`
//! model so future load tests can share the same checks.

#![allow(dead_code)]

use std::time::Instant;

/// Resident-set size (bytes) of the current process.
///
/// On Linux, reads column 2 ("resident" pages) from `/proc/self/statm`
/// and multiplies by 4096. Every Linux target we run on uses 4 KiB pages;
/// avoiding the `sysconf` dance keeps this helper libc-free.
///
/// Returns 0 if `/proc/self/statm` cannot be read (sandboxed, chroot,
/// non-Linux). Callers must treat `0` as "unavailable" and either skip
/// the assertion or sample repeatedly.
#[cfg(target_os = "linux")]
pub fn rss_bytes() -> usize {
    const PAGE_SIZE: usize = 4096;
    let statm = match std::fs::read_to_string("/proc/self/statm") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|tok| tok.parse::<usize>().ok())
        .map(|pages| pages * PAGE_SIZE)
        .unwrap_or(0)
}

/// Non-Linux fallback — returns 0; the linear-growth assertion no-ops.
#[cfg(not(target_os = "linux"))]
pub fn rss_bytes() -> usize {
    0
}

/// Assert that RSS grows at most linearly under a per-1000-event budget.
///
/// Per TECH-TUI-OBSERVABILITY line 1177 the signature is:
/// `assert_linear_memory_growth(samples: &[(Instant, usize)], max_mb_per_1k_events: f64, total_events: usize)`.
///
/// Sample-to-event mapping: this helper does not receive per-sample event
/// counts. It assumes samples are taken at **equally-spaced checkpoints**
/// that together cover `total_events`, i.e. `event_count[i] = total_events
/// * (i + 1) / samples.len()`. The TASK-TUI-813 harness collects samples
/// at 1k/2k/5k/10k which are not equally spaced — in practice the slope
/// check is a sanity bound on per-checkpoint jitter, not a tight
/// regression model. The end-to-end ratio check (step 1-3 below) remains
/// the authoritative budget gate.
///
/// Algorithm:
///   1. Compute `growth_mb` from the first and last sample.
///   2. `ratio = growth_mb / (total_events / 1000)`.
///   3. Assert `ratio <= max_mb_per_1k_events` with a message listing all
///      samples, the ratio, and the budget.
///   4. Fit a simple least-squares linear regression over
///      `(event_count, rss)` and log the slope.
///   5. Compute the mean absolute slope across consecutive pairs, then
///      assert that the max consecutive slope is within 2× the mean.
///      Catches super-linear growth between checkpoints that a coarse
///      end-to-end ratio would miss. Skipped when fewer than 2 samples
///      or when the mean slope is effectively zero.
///
/// If RSS sampling is unavailable (all `samples.1` == 0) the assertion
/// no-ops with an `eprintln!` — we prefer a noisy throughput-only signal
/// over a false failure on a platform that can't measure RSS cheaply.
///
/// Panic message includes every sample and every slope so CI logs have
/// enough data to diagnose a regression without re-running locally.
pub fn assert_linear_memory_growth(
    samples: &[(Instant, usize)],
    max_mb_per_1k_events: f64,
    total_events: usize,
) {
    if samples.len() < 2 || total_events == 0 {
        eprintln!(
            "[assert_linear_memory_growth] skipping: samples.len()={}, total_events={}",
            samples.len(),
            total_events
        );
        return;
    }
    if samples.iter().all(|(_, rss)| *rss == 0) {
        eprintln!(
            "[assert_linear_memory_growth] RSS unavailable on this platform; skipping \
             (samples.len()={}, total_events={})",
            samples.len(),
            total_events
        );
        return;
    }

    let first_rss = samples.first().unwrap().1;
    let last_rss = samples.last().unwrap().1;
    let growth_bytes = last_rss.saturating_sub(first_rss);
    let growth_mb = growth_bytes as f64 / (1024.0 * 1024.0);
    let ratio = growth_mb / (total_events as f64 / 1000.0);

    // Map sample indices -> event counts (equal-spacing assumption).
    let n = samples.len() as f64;
    let event_counts: Vec<f64> = (0..samples.len())
        .map(|i| total_events as f64 * (i as f64 + 1.0) / n)
        .collect();
    let rss_vals: Vec<f64> = samples.iter().map(|(_, rss)| *rss as f64).collect();

    // Least-squares linear regression (slope in bytes-per-event).
    let mean_x = event_counts.iter().sum::<f64>() / n;
    let mean_y = rss_vals.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..samples.len() {
        let dx = event_counts[i] - mean_x;
        num += dx * (rss_vals[i] - mean_y);
        den += dx * dx;
    }
    let regression_slope = if den > 0.0 { num / den } else { 0.0 };

    // Consecutive slopes (bytes-per-event between adjacent checkpoints).
    let mut cons_slopes: Vec<f64> = Vec::with_capacity(samples.len() - 1);
    for i in 1..samples.len() {
        let dx = event_counts[i] - event_counts[i - 1];
        let dy = rss_vals[i] - rss_vals[i - 1];
        cons_slopes.push(if dx > 0.0 { dy / dx } else { 0.0 });
    }
    let abs_mean_slope: f64 =
        cons_slopes.iter().map(|s| s.abs()).sum::<f64>() / cons_slopes.len() as f64;
    let abs_max_slope: f64 = cons_slopes
        .iter()
        .map(|s| s.abs())
        .fold(0.0_f64, f64::max);

    // Primary budget gate: end-to-end ratio must respect the MB-per-1k bound.
    assert!(
        ratio <= max_mb_per_1k_events,
        "memory growth {:.3} MB per 1000 events exceeds budget {:.3}; \
         samples={:?} regression_slope_bytes_per_event={:.3} \
         consecutive_slopes={:?} total_events={}",
        ratio,
        max_mb_per_1k_events,
        samples,
        regression_slope,
        cons_slopes,
        total_events,
    );

    // Secondary shape gate: catch super-linear blow-ups between checkpoints.
    //
    // Gated on an **absolute floor tied to allocator granularity**, not a
    // fraction of the primary budget. Per Sherlock Gate 3 review
    // (a038dcf94e74a897d), a budget-proportional gate (e.g. 25% of
    // `max_mb_per_1k_events × total_events`) scales with the budget and
    // lets concentrated super-linear leaks hide under a generous budget —
    // a 40 MiB blow-up could both pass the primary ratio gate AND skip the
    // shape gate at budget=50 MB/1k × 10k events (125 MiB threshold).
    //
    // Absolute floor rationale: glibc's heap grows in ~1 MiB steps; 4 MiB
    // is 4× that to stay clear of page-step false positives under small
    // loads, while still ensuring any real super-linear leak of 4+ MiB
    // triggers the shape check regardless of how generous the primary
    // budget is.
    const ALLOCATOR_GRANULARITY_MB: f64 = 4.0;
    if growth_mb >= ALLOCATOR_GRANULARITY_MB && abs_mean_slope > 1.0 {
        assert!(
            abs_max_slope <= 2.0 * abs_mean_slope,
            "non-linear growth detected: max consecutive slope {:.3} > 2× mean slope {:.3}; \
             samples={:?} consecutive_slopes={:?} regression_slope={:.3} \
             total_events={} growth_mb={:.3}",
            abs_max_slope,
            abs_mean_slope,
            samples,
            cons_slopes,
            regression_slope,
            total_events,
            growth_mb,
        );
    }

    eprintln!(
        "[assert_linear_memory_growth] ratio={:.3} MB/1k (budget {:.3}), \
         regression_slope={:.3} bytes/event, mean_consec_slope={:.3}, \
         max_consec_slope={:.3}, samples={:?}",
        ratio, max_mb_per_1k_events, regression_slope, abs_mean_slope, abs_max_slope, samples,
    );
}
