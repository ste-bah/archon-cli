//! discovery_scan bench — NFR-PERF-002 (<1 s scan of 300-agent fixture).
//!
//! Builds a 300-file `.md` agent fixture in a TempDir at startup (each
//! file is minimal valid frontmatter with `name`+`description`), then
//! repeatedly invokes `archon_pipeline::agent_loader::load_coding_agents`
//! to measure the discovery scan time. Records per-iteration latency
//! into hdrhistogram, asserts p95 against threshold.toml.
//!
//! Why we don't use scripts/gen-discovery-fixture.sh: that script writes
//! `.yaml` files with no frontmatter delimiters, but the production loader
//! globs `*.md` and parses `---`-delimited frontmatter. Pointing the loader
//! at the script's output yields zero loaded agents (silent skip). The
//! script targets a future phase-3 scanner that does not yet exist in Rust.
//! Building the fixture in-bench keeps the measurement on the real
//! production code path.
//!
//! Iteration count: 30 — discovery is slower per-iter than task_submit
//! (300 file reads + YAML parses), so we cap iterations to keep wall-clock
//! reasonable while still giving the hdrhistogram enough samples.

use archon_bench::{p95, thresholds};
use archon_pipeline::agent_loader::load_coding_agents;
use criterion::{Criterion, criterion_group, criterion_main};
use std::time::{Duration, Instant};

const NUM_AGENTS: usize = 300;
const ITER: usize = 30;

fn write_fixture(dir: &std::path::Path) {
    for i in 0..NUM_AGENTS {
        let filename = format!("agent_{i:03}.md");
        let content = format!(
            "---\n\
             name: bench-agent-{i:03}\n\
             description: Bench fixture agent {i:03} for discovery_scan NFR-PERF-002.\n\
             ---\n\
             Synthetic prompt body for bench agent {i:03}.\n"
        );
        std::fs::write(dir.join(&filename), content).expect("write fixture file");
    }
}

fn bench_discovery_scan(_c: &mut Criterion) {
    let fixture_dir = tempfile::TempDir::new().expect("create fixture tempdir");
    write_fixture(fixture_dir.path());

    // Sanity check: confirm the loader actually parses every file before
    // we start timing. A silent skip (returning 0 agents) would make
    // every iteration trivially fast and hide a regression.
    let initial = load_coding_agents(fixture_dir.path()).expect("initial load must succeed");
    assert_eq!(
        initial.len(),
        NUM_AGENTS,
        "fixture sanity check: expected {NUM_AGENTS} agents, loaded {}",
        initial.len()
    );

    let mut durations: Vec<Duration> = Vec::with_capacity(ITER);
    for _ in 0..ITER {
        let t0 = Instant::now();
        let agents = load_coding_agents(fixture_dir.path()).expect("load_coding_agents");
        let elapsed = t0.elapsed();
        // Keep the loader's result alive past the timing boundary so the
        // optimiser cannot elide the work.
        std::hint::black_box(&agents);
        durations.push(elapsed);
    }

    let p95_ms = p95::p95_ms(&durations);
    let threshold_ms = thresholds::get_p95_ms("discovery_scan");

    let tmp_artifact = tempfile::TempDir::new().expect("artifact tempdir");
    let json_path = tmp_artifact.path().join("discovery_scan.json");
    let mean_us: u128 =
        durations.iter().map(|d| d.as_micros()).sum::<u128>() / durations.len() as u128;
    let json = serde_json::json!({
        "bench": "discovery_scan",
        "nfr": "NFR-PERF-002",
        "iterations": ITER,
        "fixture_agents": NUM_AGENTS,
        "p95_ms": p95_ms,
        "mean_us": mean_us,
        "threshold_p95_ms": threshold_ms,
        "passed": p95_ms <= threshold_ms,
        "timestamp_utc": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap()).ok();

    assert!(
        p95_ms <= threshold_ms,
        "discovery_scan p95 {p95_ms}ms exceeds threshold {threshold_ms}ms \
         (NFR-PERF-002; mean {mean_us}us across {ITER} iterations, \
         {NUM_AGENTS} agents/iter)"
    );
}

criterion_group!(benches, bench_discovery_scan);
criterion_main!(benches);
