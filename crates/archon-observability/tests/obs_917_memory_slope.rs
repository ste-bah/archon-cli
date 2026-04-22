//! OBS-917: Memory-growth slope integration test.
//!
//! Measures resident-set size (VmRSS) growth while repeatedly exercising
//! the redaction layer under a realistic event load. A positive slope
//! above the threshold indicates a memory leak in the tracing /
//! redaction stack.
//!
//! Linux-only: `/proc/self/status` is the only portable (within Linux)
//! source for VmRSS. On other platforms the test module is compiled out.

#[cfg(target_os = "linux")]
mod linux {
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

    #[test]
    fn obs_917_memory_slope() {
        // Tunables
        const WARMUP_ITERATIONS: usize = 100;
        const SAMPLED_ITERATIONS: usize = 500;
        const SAMPLE_EVERY: usize = 50;
        const EVENTS_PER_ITERATION: usize = 100;

        // Threshold rationale: 1024 bytes/iteration ≈ 10 bytes/event.
        // This is generous enough to absorb allocator noise, page-granular
        // RSS jitter, and jemalloc arena retention, while still catching
        // unbounded growth (e.g., a subscriber leak or regex cache bloat).
        const SLOPE_THRESHOLD_BYTES_PER_ITER: f64 = 1024.0;

        // Warmup: let the lazy regex compilation, tracing-subscriber
        // thread-local caches, and allocator arenas reach steady state so
        // they do not inflate the regression slope.
        for _ in 0..WARMUP_ITERATIONS {
            run_batch(EVENTS_PER_ITERATION);
        }

        let mut samples: Vec<(f64, f64)> = Vec::new();

        // Baseline sample after warmup.
        samples.push((0.0, read_vmrss_bytes() as f64));

        for iter in 1..=SAMPLED_ITERATIONS {
            // Workload: install a fresh redaction layer and emit events.
            // We use `with_default` rather than `init_tracing` so the
            // subscriber (and all internal allocations) are dropped after each
            // batch, isolating per-iteration memory behaviour.
            run_batch(EVENTS_PER_ITERATION);

            if iter % SAMPLE_EVERY == 0 {
                let bytes = read_vmrss_bytes();
                samples.push((iter as f64, bytes as f64));
            }
        }

        // Ensure we have at least 2 samples to fit a line.
        assert!(
            samples.len() >= 2,
            "need at least 2 memory samples, got {}",
            samples.len()
        );

        // Least-squares linear regression: y = slope * x + intercept
        let n = samples.len() as f64;
        let sum_x: f64 = samples.iter().map(|(x, _)| x).sum();
        let sum_y: f64 = samples.iter().map(|(_, y)| y).sum();
        let sum_xy: f64 = samples.iter().map(|(x, y)| x * y).sum();
        let sum_x2: f64 = samples.iter().map(|(x, _)| x * x).sum();

        let denominator = n * sum_x2 - sum_x * sum_x;
        assert!(
            denominator.abs() > 1e-12,
            "regression denominator too small (degenerate samples)"
        );

        let slope = (n * sum_xy - sum_x * sum_y) / denominator;
        let intercept = (sum_y - slope * sum_x) / n;

        // Coefficient of determination R²
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

        // Diagnostic output
        eprintln!("\n=== OBS-917 Memory Slope Report ===");
        eprintln!("samples (iteration -> bytes):");
        for (x, y) in &samples {
            eprintln!("  iter {:>6.0} -> {:>12.0} bytes", x, y);
        }
        eprintln!("slope:     {:>12.2} bytes/iteration", slope);
        eprintln!("intercept: {:>12.2} bytes", intercept);
        eprintln!("R²:        {:>12.4}", r2);
        eprintln!("====================================\n");

        // R² sanity check: if the regression is too noisy the slope is not
        // trustworthy. A very low R² usually means RSS is flat (good) but
        // with page-granular jitter.
        assert!(
            r2 > 0.5,
            "regression too noisy to trust slope (R² = {r2}); consider re-running"
        );

        assert!(
            slope < SLOPE_THRESHOLD_BYTES_PER_ITER,
            "memory growth slope {:.2} bytes/iteration exceeds threshold {:.2}",
            slope,
            SLOPE_THRESHOLD_BYTES_PER_ITER
        );
    }
}
