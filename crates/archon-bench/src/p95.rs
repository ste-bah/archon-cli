//! In-bench p95 computation via hdrhistogram.
//!
//! Criterion's `estimates.json` reports only mean/median/MAD/std_dev/slope
//! — it does not expose percentile points. NFR-PERF gates are stated in
//! terms of p95 latency, so each bench records per-iteration durations into
//! an [`hdrhistogram::Histogram`] and queries the 95th percentile itself.

use hdrhistogram::Histogram;
use std::time::Duration;

/// Compute the 95th percentile of a sample slice, rounded up to whole
/// milliseconds.
///
/// The histogram is configured for a 1-microsecond minimum and a
/// 1-hour maximum with 3 significant figures of precision; outside that
/// range the sample is clamped to the boundary, which is the desired
/// failure mode for an NFR gate (the assertion will still fire).
pub fn p95_ms(samples: &[Duration]) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let mut hist: Histogram<u64> = Histogram::new_with_bounds(1, 3_600_000_000, 3)
        .expect("hdrhistogram bounds are valid");

    for d in samples {
        let micros = d.as_micros().min(u64::MAX as u128) as u64;
        // Clamp into the valid range; values outside still feed into
        // the upper / lower bucket boundary.
        let clamped = micros.clamp(1, 3_600_000_000);
        // Record errors are only returned for values outside [low, high].
        // Clamping above eliminates that branch.
        hist.record(clamped).expect("clamped sample is in range");
    }

    // hdrhistogram returns the value at the 95th percentile in microseconds
    // (our recorded unit). Convert to milliseconds, rounding up so we never
    // pretend a sample is faster than its measurement.
    let p95_us = hist.value_at_percentile(95.0);
    p95_us.div_ceil(1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_zero() {
        assert_eq!(p95_ms(&[]), 0);
    }

    #[test]
    fn uniform_samples_return_the_uniform_value() {
        let samples = vec![Duration::from_millis(50); 100];
        // hdrhistogram clusters into buckets at 3 sig figs; expect within 1ms.
        let got = p95_ms(&samples);
        assert!(
            (49..=51).contains(&got),
            "expected ~50ms p95, got {got}ms"
        );
    }

    #[test]
    fn p95_captures_upper_tail() {
        // 900 fast at 10ms + 100 slow at 500ms = 1000 samples.
        // The top 10% are slow, so the 95th percentile is well inside the
        // slow region. Using a wider gap than 95/5 of 100 samples avoids
        // hdrhistogram's percentile-convention boundary case.
        let mut samples = vec![Duration::from_millis(10); 900];
        samples.extend(vec![Duration::from_millis(500); 100]);
        let got = p95_ms(&samples);
        assert!(
            got >= 100,
            "expected p95 to capture the slow tail, got {got}ms"
        );
    }
}
