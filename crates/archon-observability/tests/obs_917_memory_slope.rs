//! OBS-917: Memory-growth slope integration test.
//!
//! Measures resident-set size (VmRSS) growth while repeatedly exercising
//! the redaction layer under a realistic event load. A positive slope
//! above the threshold indicates a memory leak in the tracing /
//! redaction stack.
//!
//! Linux-only: `/proc/self/status` is the only portable (within Linux)
//! source for VmRSS. On other platforms the test module is compiled out.

const SLOPE_THRESHOLD_BYTES_PER_ITER: f64 = 256.0;
const TRUSTWORTHY_R2: f64 = 0.5;

#[derive(Debug, PartialEq)]
enum SlopeDecision {
    Leak { slope: f64, r2: f64 },
    Flat { slope: f64, r2: f64 },
    NoisyAboveThreshold { slope: f64, r2: f64 },
}

#[derive(Debug, PartialEq)]
enum SeriesDecision {
    Pass,
    Retry,
    Fail,
}

fn decide_slope(samples: &[(f64, f64)]) -> SlopeDecision {
    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|(x, _)| x).sum();
    let sum_y: f64 = samples.iter().map(|(_, y)| y).sum();
    let sum_xy: f64 = samples.iter().map(|(x, y)| x * y).sum();
    let sum_x2: f64 = samples.iter().map(|(x, _)| x * x).sum();
    let denominator = n * sum_x2 - sum_x * sum_x;
    assert!(n >= 2.0 && denominator.abs() > 1e-12, "degenerate samples");

    let slope = (n * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - slope * sum_x) / n;
    let y_mean = sum_y / n;
    let ss_tot: f64 = samples.iter().map(|(_, y)| (y - y_mean).powi(2)).sum();
    let ss_res: f64 = samples
        .iter()
        .map(|(x, y)| (y - (slope * x + intercept)).powi(2))
        .sum();
    let r2 = if ss_tot.abs() < 1e-12 {
        1.0
    } else {
        1.0 - (ss_res / ss_tot)
    };

    if slope < SLOPE_THRESHOLD_BYTES_PER_ITER {
        SlopeDecision::Flat { slope, r2 }
    } else if r2 > TRUSTWORTHY_R2 {
        SlopeDecision::Leak { slope, r2 }
    } else {
        SlopeDecision::NoisyAboveThreshold { slope, r2 }
    }
}

fn decide_series(samples: &[&[(f64, f64)]]) -> SeriesDecision {
    assert!(!samples.is_empty(), "need at least one sample series");
    match decide_slope(samples[0]) {
        SlopeDecision::Flat { .. } => SeriesDecision::Pass,
        SlopeDecision::Leak { .. } => SeriesDecision::Fail,
        SlopeDecision::NoisyAboveThreshold { .. } if samples.len() == 1 => SeriesDecision::Retry,
        SlopeDecision::NoisyAboveThreshold { .. } => match decide_slope(samples[1]) {
            SlopeDecision::Flat { .. } => SeriesDecision::Pass,
            SlopeDecision::Leak { .. } | SlopeDecision::NoisyAboveThreshold { .. } => {
                SeriesDecision::Fail
            }
        },
    }
}

#[test]
fn flat_page_size_jitter_passes() {
    let samples = [
        (0.0, 0.0),
        (50.0, 4096.0),
        (100.0, 0.0),
        (150.0, 4096.0),
        (200.0, 0.0),
        (250.0, 4096.0),
        (300.0, 0.0),
        (350.0, 4096.0),
        (400.0, 0.0),
        (450.0, 4096.0),
        (500.0, 0.0),
    ];
    assert_eq!(decide_series(&[&samples]), SeriesDecision::Pass);
}

#[test]
fn steady_growth_above_threshold_fails() {
    let samples = [
        (0.0, 0.0),
        (50.0, 20_000.0),
        (100.0, 40_000.0),
        (150.0, 60_000.0),
        (200.0, 80_000.0),
        (250.0, 100_000.0),
    ];
    assert_eq!(decide_series(&[&samples]), SeriesDecision::Fail);
}

#[test]
fn noisy_above_threshold_twice_fails() {
    let samples = [
        (0.0, 6_000_000.0),
        (50.0, 4_250_000.0),
        (100.0, 6_500_000.0),
        (150.0, 4_750_000.0),
        (200.0, 7_000_000.0),
        (250.0, 5_250_000.0),
        (300.0, 7_500_000.0),
        (350.0, 5_750_000.0),
        (400.0, 8_000_000.0),
        (450.0, 6_250_000.0),
        (500.0, 8_500_000.0),
    ];
    assert_eq!(decide_series(&[&samples]), SeriesDecision::Retry);
    assert_eq!(decide_series(&[&samples, &samples]), SeriesDecision::Fail);
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{SeriesDecision, SlopeDecision, decide_series, decide_slope};
    use std::io::Write;
    use tracing_subscriber::layer::SubscriberExt;

    /// No-op writer so the test measures only the redaction / tracing
    /// memory behaviour, not unbounded Vec growth in the sink.
    struct NullWriter;

    impl Write for NullWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Read VmRSS from `/proc/self/status` and return it in **bytes**.
    ///
    /// Panics if the file is missing (non-Linux) or the `VmRSS:` line is
    /// absent / malformed. The unit token is verified to be `kB` so a
    /// future kernel change does not silently corrupt the measurement.
    fn read_vmrss_bytes() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status")
            .expect("/proc/self/status must be readable on Linux");
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                assert_eq!(
                    parts.get(2),
                    Some(&"kB"),
                    "VmRSS unit changed; parser needs update: {line}"
                );
                let kb: u64 = parts[1]
                    .parse()
                    .expect("VmRSS value must be a positive integer");
                return kb * 1024;
            }
        }
        panic!("VmRSS line not found in /proc/self/status");
    }

    /// Run one workload batch. Extracted so the warmup and sampled phases
    /// execute exactly the same code path.
    fn run_batch(events: usize) {
        let layer = archon_observability::RedactionLayer::with_writer(NullWriter);
        let subscriber = tracing_subscriber::registry().with(layer);

        ::tracing::subscriber::with_default(subscriber, || {
            for i in 0..events {
                // Rotate through secret shapes so the regex alternation is
                // fully exercised.
                let secret = match i % 9 {
                    0 => "sk-ant-api03_ZZZZZZZZZZZZZZZZZZ1234",
                    1 => "sk-abcdefghijklmnopqrst0000",
                    2 => "AKIAZZZZZZZZZZZZZZZZ",
                    3 => "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                    4 => "gho_abcdefghijklmnopqrstuvwxyz0123456789",
                    5 => "sk_live_abcdefghijklmnopqrstuvwx",
                    6 => "pk_live_abcdefghijklmnopqrstuvwx",
                    7 => "eyJhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe",
                    _ => "bearer ya29.a0Af_abcDEF-123",
                };

                ::tracing::info!(
                    api_key = %secret,
                    password = "hunter2",
                    token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                    iteration = i,
                    "workload event with redaction exercise"
                );
            }
        });
    }

    fn collect_samples(
        sampled_iterations: usize,
        sample_every: usize,
        events_per_iteration: usize,
    ) -> Vec<(f64, f64)> {
        let mut samples = vec![(0.0, read_vmrss_bytes() as f64)];
        for iter in 1..=sampled_iterations {
            // Each iteration installs and drops a fresh subscriber.
            run_batch(events_per_iteration);
            if iter % sample_every == 0 {
                samples.push((iter as f64, read_vmrss_bytes() as f64));
            }
        }
        samples
    }

    fn print_report(series_number: usize, samples: &[(f64, f64)], decision: &SlopeDecision) {
        eprintln!("\n=== Memory Slope Series {series_number} ===");
        eprintln!("samples (iteration -> bytes):");
        for (x, y) in samples {
            eprintln!("  iter {:>6.0} -> {:>12.0} bytes", x, y);
        }
        match decision {
            SlopeDecision::Leak { slope, r2 }
            | SlopeDecision::Flat { slope, r2 }
            | SlopeDecision::NoisyAboveThreshold { slope, r2 } => {
                eprintln!("slope: {:>12.2} bytes/iteration", slope);
                eprintln!("R²:    {:>12.4}", r2);
            }
        }
        eprintln!("====================================\n");
    }

    #[test]
    fn obs_917_memory_slope() {
        // Tunables
        const WARMUP_ITERATIONS: usize = 100;
        const SAMPLED_ITERATIONS: usize = 500;
        const SAMPLE_EVERY: usize = 50;
        const EVENTS_PER_ITERATION: usize = 100;

        // Threshold rationale: 256 bytes/iteration ≈ 2.5 bytes/event — 4x
        // tighter than the pre-OBS-917-TIGHTEN budget of 1024 bytes/iter.
        // Baseline empirical measurement on this workload observes slope
        // 0.00 bytes/iter (flat RSS), so 256 leaves ample headroom for
        // allocator noise, page-granular RSS jitter, and jemalloc arena
        // retention while catching even small subscriber / regex-cache
        // leaks (per-event regressions >= ~3 bytes are now surfaced).
        // Warmup: let the lazy regex compilation, tracing-subscriber
        // thread-local caches, and allocator arenas reach steady state so
        // they do not inflate the regression slope.
        for _ in 0..WARMUP_ITERATIONS {
            run_batch(EVENTS_PER_ITERATION);
        }

        let first = collect_samples(SAMPLED_ITERATIONS, SAMPLE_EVERY, EVENTS_PER_ITERATION);
        assert!(
            first.len() >= 2,
            "need at least 2 memory samples, got {}",
            first.len()
        );
        let first_decision = decide_slope(&first);
        print_report(1, &first, &first_decision);

        match decide_series(&[&first]) {
            SeriesDecision::Pass => {}
            SeriesDecision::Fail => {
                panic!("memory growth slope exceeds threshold: {first_decision:?}")
            }
            SeriesDecision::Retry => {
                let second =
                    collect_samples(SAMPLED_ITERATIONS, SAMPLE_EVERY, EVENTS_PER_ITERATION);
                assert!(
                    second.len() >= 2,
                    "need at least 2 memory samples, got {}",
                    second.len()
                );
                let second_decision = decide_slope(&second);
                print_report(2, &second, &second_decision);
                assert_eq!(
                    decide_series(&[&first, &second]),
                    SeriesDecision::Pass,
                    "memory slope remained above threshold or untrustworthy after two series; first: {first_decision:?}; second: {second_decision:?}; first samples: {first:?}; second samples: {second:?}"
                );
            }
        }
    }
}
