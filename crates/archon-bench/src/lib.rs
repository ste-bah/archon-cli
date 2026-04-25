//! archon-bench harness crate — populated by phase-1..3 tasks.
//!
//! The three benches live under `benches/`:
//!
//! - `task_submit`    — NFR-PERF-001 (<100 ms p95 submit)
//! - `discovery_scan` — NFR-PERF-002 (<1 s scan over 300 agents)
//! - `fanout_100`     — NFR-SCALABILITY-001 (100 workers <1 s)
//!
//! Latency/throughput limits live in `threshold.toml` and are the
//! single source of truth for "what counts as a regression".
