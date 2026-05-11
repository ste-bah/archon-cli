//! fanout_100 bench — NFR-SCALABILITY-001 (100 workers reach RUNNING <1 s).
//!
//! Dispatches 100 stub workers through `FanOutFanInPattern` and measures
//! the time from `pattern.execute(...)` start until the LAST of the 100
//! workers has been invoked (proxy for "reach RUNNING": the moment the
//! pattern dispatches a worker is when that worker would transition to
//! TaskState::Running in the real executor at executor.rs:215).
//!
//! Proxy chosen because `TaskServiceHandle` does not expose status
//! observation — `submit()` returns a value (completion), not a state
//! transition. Recording `Instant::now()` at the top of the stub's
//! `submit()` is the closest semantic equivalent and matches the
//! research synthesis (a30b12de0e20c4cac).
//!
//! Each iteration runs one full fanout (100 workers + 1 aggregator).
//! The aggregator is the 101st call into the stub; we filter it out by
//! taking only the first 100 timestamps (workers are dispatched in
//! parallel via `buffer_unordered(100)`, then the aggregator runs once
//! after fan-in).
//!
//! Iteration count: 100 — gives the hdrhistogram enough samples for a
//! stable p95 across the per-iteration "max worker dispatch latency".

use archon_bench::{p95, thresholds};
use archon_core::patterns::fanout::FanOutFanInPattern;
use archon_core::patterns::spec::FanOutConfig;
use archon_core::patterns::{
    Pattern, PatternCtx, PatternError, PatternRegistry, TaskServiceHandle,
};
use async_trait::async_trait;
use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Builder;

const NUM_WORKERS: usize = 100;
const ITER: usize = 100;

/// Stub TaskServiceHandle that records a timestamp at the top of every
/// `submit()` call. The first `NUM_WORKERS` entries correspond to fan-out
/// worker dispatches; the trailing entry (if any) is the aggregator.
struct LatencyStub {
    starts: Mutex<Vec<Instant>>,
}

impl LatencyStub {
    fn new() -> Self {
        Self {
            starts: Mutex::new(Vec::with_capacity(NUM_WORKERS + 1)),
        }
    }

    fn take_starts(&self) -> Vec<Instant> {
        let mut guard = self.starts.lock().expect("starts mutex");
        std::mem::take(&mut *guard)
    }
}

#[async_trait]
impl TaskServiceHandle for LatencyStub {
    async fn submit(&self, _agent: &str, _input: Value) -> Result<Value, PatternError> {
        self.starts
            .lock()
            .expect("starts mutex")
            .push(Instant::now());
        Ok(json!({}))
    }
}

fn make_ctx(svc: Arc<dyn TaskServiceHandle>) -> PatternCtx {
    PatternCtx {
        task_service: svc,
        registry: Arc::new(PatternRegistry::new()),
        trace_id: "fanout_100_bench".into(),
        deadline: None,
    }
}

fn bench_fanout_100(_c: &mut Criterion) {
    let rt = Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("build multi-thread tokio runtime");

    let workers: Vec<String> = (0..NUM_WORKERS).map(|i| format!("w{i}")).collect();
    let cfg = FanOutConfig {
        workers,
        aggregator: "agg".into(),
        partition_fn: None,
    };
    let pattern = FanOutFanInPattern::new(cfg);

    rt.block_on(async {
        let mut durations: Vec<Duration> = Vec::with_capacity(ITER);
        for _ in 0..ITER {
            let stub = Arc::new(LatencyStub::new());
            let ctx = make_ctx(stub.clone());
            let t0 = Instant::now();
            let _ = pattern
                .execute(json!({"data": 0}), ctx)
                .await
                .expect("fanout execute");

            let mut starts = stub.take_starts();
            assert!(
                starts.len() >= NUM_WORKERS,
                "expected at least {NUM_WORKERS} starts, got {}",
                starts.len()
            );
            // Take the first NUM_WORKERS starts (workers, dispatched in
            // parallel before the aggregator runs).
            starts.truncate(NUM_WORKERS);
            let last_worker_start = starts
                .iter()
                .max()
                .copied()
                .expect("at least one worker start");
            durations.push(last_worker_start.duration_since(t0));
        }

        let p95_ms = p95::p95_ms(&durations);
        let threshold_ms = thresholds::get_p95_ms("fanout_100");

        let tmp_artifact = tempfile::TempDir::new().expect("artifact tempdir");
        let json_path = tmp_artifact.path().join("fanout_100.json");
        let mean_us: u128 =
            durations.iter().map(|d| d.as_micros()).sum::<u128>() / durations.len() as u128;
        let json = serde_json::json!({
            "bench": "fanout_100",
            "nfr": "NFR-SCALABILITY-001",
            "iterations": ITER,
            "workers_per_iter": NUM_WORKERS,
            "p95_ms": p95_ms,
            "mean_us": mean_us,
            "threshold_p95_ms": threshold_ms,
            "passed": p95_ms <= threshold_ms,
            "timestamp_utc": chrono::Utc::now().to_rfc3339(),
        });
        std::fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap()).ok();

        assert!(
            p95_ms <= threshold_ms,
            "fanout_100 p95 {p95_ms}ms exceeds threshold {threshold_ms}ms \
             (NFR-SCALABILITY-001; mean {mean_us}us across {ITER} iterations, \
             {NUM_WORKERS} workers/iter)"
        );
    });
}

criterion_group!(benches, bench_fanout_100);
criterion_main!(benches);
