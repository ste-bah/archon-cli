//! task_submit bench — NFR-PERF-001 (<100 ms p95 submit path).
//!
//! Measures DefaultTaskService::submit() end-to-end against the in-memory
//! task store and a TempDir-anchored AgentRegistry (built-in agents only;
//! the `general-purpose` agent resolves without any filesystem population).
//! Records per-iteration latencies into hdrhistogram, asserts p95 against
//! threshold.toml at end. Writes a JSON artifact to a tempdir for inspection.
//!
//! Iteration count: 200 — enough samples for stable p95, small enough to
//! not blow up the tasks DashMap. The map grows monotonically across the
//! loop, so any hash-resize cost is part of the measured submit path
//! (representative of production behaviour).

use archon_bench::{p95, thresholds};
use archon_core::agents::registry::AgentRegistry;
use archon_core::tasks::models::SubmitRequest;
use archon_core::tasks::service::{DefaultTaskService, TaskService};
use criterion::{Criterion, criterion_group, criterion_main};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Builder;

fn bench_task_submit(_c: &mut Criterion) {
    const ITER: usize = 200;

    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let registry = Arc::new(AgentRegistry::load(tmp.path()));
        let svc = DefaultTaskService::new(registry, usize::MAX);

        let mut durations: Vec<Duration> = Vec::with_capacity(ITER);

        for _ in 0..ITER {
            let req = SubmitRequest {
                agent_name: "general-purpose".to_string(),
                agent_version: None,
                input: serde_json::json!({"prompt": "bench"}),
                owner: "bench".to_string(),
            };
            let t0 = Instant::now();
            let _ = svc.submit(req).await.expect("submit");
            durations.push(t0.elapsed());
        }

        let p95_ms = p95::p95_ms(&durations);
        let threshold_ms = thresholds::get_p95_ms("task_submit");

        // JSON artifact (matches existing TUI-bench convention).
        let tmp_artifact = tempfile::TempDir::new().expect("artifact tempdir");
        let json_path = tmp_artifact.path().join("task_submit.json");
        let mean_us: u128 =
            durations.iter().map(|d| d.as_micros()).sum::<u128>() / durations.len() as u128;
        let json = serde_json::json!({
            "bench": "task_submit",
            "nfr": "NFR-PERF-001",
            "iterations": ITER,
            "p95_ms": p95_ms,
            "mean_us": mean_us,
            "threshold_p95_ms": threshold_ms,
            "passed": p95_ms <= threshold_ms,
            "timestamp_utc": chrono::Utc::now().to_rfc3339(),
        });
        std::fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap()).ok();

        assert!(
            p95_ms <= threshold_ms,
            "task_submit p95 {p95_ms}ms exceeds threshold {threshold_ms}ms \
             (NFR-PERF-001; mean {mean_us}us across {ITER} iterations)"
        );
    });
}

criterion_group!(benches, bench_task_submit);
criterion_main!(benches);
