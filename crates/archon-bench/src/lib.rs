//! archon-bench harness crate.
//!
//! The three benches live under `benches/`:
//!
//! - `task_submit`    — NFR-PERF-001 (<100 ms p95 submit)
//! - `discovery_scan` — NFR-PERF-002 (<1 s scan over 300 agents)
//! - `fanout_100`     — NFR-SCALABILITY-001 (100 workers <1 s)
//!
//! Latency limits live in `threshold.toml` and are the single source of
//! truth for "what counts as a regression". Benches read them via
//! [`thresholds::get_p95_ms`] at startup.
//!
//! Criterion's `estimates.json` does not store percentile points
//! (only mean, median, MAD, std_dev, slope). Each bench therefore
//! records per-iteration durations into an [`hdrhistogram::Histogram`]
//! and asserts the p95 itself via [`p95::p95_ms`].

pub mod p95;
pub mod thresholds;
