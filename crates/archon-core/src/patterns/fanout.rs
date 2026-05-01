//! TASK-AGS-502: FanOutFanInPattern — parallel distribution + aggregator merge.
//!
//! Distributes input to N worker agents in parallel via `TaskServiceHandle`,
//! then merges results through a coordinator (aggregator) agent.
//! Uses `futures_util::stream::iter(...).buffer_unordered(cap)` bounded
//! by a hard cap of 100 (NFR-SCALABILITY-001).
//!
//! NO `tokio::spawn` — all dispatch goes through `TaskServiceHandle`.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};

use super::{FanOutConfig, Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry};

/// Maximum number of concurrent workers (NFR-SCALABILITY-001).
const MAX_PARALLELISM: usize = 100;

// ---------------------------------------------------------------------------
// FanOutFanInPattern
// ---------------------------------------------------------------------------

/// Parallel fan-out to N workers, fan-in via an aggregator agent.
///
/// Zero-field struct — reads everything from `PatternCtx` + config.
pub struct FanOutFanInPattern {
    config: FanOutConfig,
}

impl FanOutFanInPattern {
    pub fn new(config: FanOutConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Pattern for FanOutFanInPattern {
    fn kind(&self) -> PatternKind {
        PatternKind::FanOut
    }

    #[allow(clippy::redundant_iter_cloned)]
    async fn execute(&self, input: Value, ctx: PatternCtx) -> Result<Value, PatternError> {
        let cfg = &self.config;
        let parallelism = cfg.workers.len().min(MAX_PARALLELISM);

        // Fan-out: dispatch to all workers in parallel, bounded.
        let task_svc = ctx.task_service.clone();
        let worker_input = input.clone();

        let results: Vec<Result<Value, PatternError>> =
            stream::iter(cfg.workers.iter().cloned().map(move |worker_name| {
                let svc = task_svc.clone();
                let inp = worker_input.clone();
                async move { svc.submit(&worker_name, inp).await }
            }))
            .buffer_unordered(parallelism)
            .collect()
            .await;

        // Partition into successes and failures.
        let mut oks: Vec<Value> = Vec::new();
        let mut errs: Vec<String> = Vec::new();

        for result in results {
            match result {
                Ok(v) => oks.push(v),
                Err(e) => errs.push(e.to_string()),
            }
        }

        // All failed → EC-ARCH-001.
        if oks.is_empty() && !errs.is_empty() {
            return Err(PatternError::Execution(format!(
                "fan-out: all {} workers failed: {}",
                cfg.workers.len(),
                errs.join("; ")
            )));
        }

        // Call aggregator with available results.
        let aggregator_input = json!({ "results": oks });
        let aggregator_output = ctx
            .task_service
            .submit(&cfg.aggregator, aggregator_input)
            .await?;

        // Some failed → EC-ARCH-002: return PartialResult.
        if !errs.is_empty() {
            return Err(PatternError::PartialResult {
                merged: aggregator_output,
                errors: errs,
            });
        }

        // All succeeded.
        Ok(aggregator_output)
    }
}

/// Register a FanOutFanInPattern into the registry under name "fanout".
pub fn register(reg: &PatternRegistry, config: FanOutConfig) {
    reg.register("fanout", Arc::new(FanOutFanInPattern::new(config)));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::json;

    use crate::patterns::{PatternRegistry, TaskServiceHandle};

    // Stub TaskService: tracks calls, optionally fails specific workers.
    struct StubTaskService {
        call_count: AtomicUsize,
        fail_workers: Vec<String>,
    }

    impl StubTaskService {
        fn new(fail_workers: Vec<String>) -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                fail_workers,
            }
        }
    }

    #[async_trait]
    impl TaskServiceHandle for StubTaskService {
        async fn submit(&self, agent: &str, input: Value) -> Result<Value, PatternError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            if self.fail_workers.contains(&agent.to_string()) {
                return Err(PatternError::Execution(format!("{agent} failed")));
            }
            // Workers return their name + input. Aggregator merges.
            Ok(json!({"agent": agent, "input": input}))
        }
    }

    fn make_ctx(svc: Arc<dyn TaskServiceHandle>) -> PatternCtx {
        PatternCtx {
            task_service: svc,
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test".into(),
            deadline: None,
        }
    }

    #[tokio::test]
    async fn test_fanout_3_workers_all_ok_aggregates() {
        let svc = Arc::new(StubTaskService::new(vec![]));
        let config = FanOutConfig {
            workers: vec!["w1".into(), "w2".into(), "w3".into()],
            aggregator: "agg".into(),
            partition_fn: None,
        };
        let pattern = FanOutFanInPattern::new(config);
        let ctx = make_ctx(svc.clone());

        let result = pattern.execute(json!({"data": 1}), ctx).await.unwrap();

        // Aggregator was called (total calls = 3 workers + 1 aggregator = 4).
        assert_eq!(svc.call_count.load(Ordering::SeqCst), 4);
        // Result is the aggregator's output.
        assert!(result.get("agent").is_some());
    }

    #[tokio::test]
    async fn test_fanout_partial_failure_returns_partial_result() {
        // EC-ARCH-002: 1 of 3 fails → PartialResult.
        let svc = Arc::new(StubTaskService::new(vec!["w2".into()]));
        let config = FanOutConfig {
            workers: vec!["w1".into(), "w2".into(), "w3".into()],
            aggregator: "agg".into(),
            partition_fn: None,
        };
        let pattern = FanOutFanInPattern::new(config);
        let ctx = make_ctx(svc);

        let err = pattern.execute(json!({"data": 1}), ctx).await.unwrap_err();
        match err {
            PatternError::PartialResult { merged, errors } => {
                assert!(!errors.is_empty(), "must report failed workers");
                assert!(merged.get("agent").is_some(), "aggregator still called");
            }
            other => panic!("expected PartialResult, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_fanout_all_fail_returns_execution_error() {
        // EC-ARCH-001: all 3 fail.
        let svc = Arc::new(StubTaskService::new(vec![
            "w1".into(),
            "w2".into(),
            "w3".into(),
        ]));
        let config = FanOutConfig {
            workers: vec!["w1".into(), "w2".into(), "w3".into()],
            aggregator: "agg".into(),
            partition_fn: None,
        };
        let pattern = FanOutFanInPattern::new(config);
        let ctx = make_ctx(svc);

        let err = pattern.execute(json!({"data": 1}), ctx).await.unwrap_err();
        match err {
            PatternError::Execution(msg) => {
                assert!(
                    msg.contains("all") && msg.contains("failed"),
                    "message must indicate all failed: {msg}"
                );
            }
            other => panic!("expected Execution, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_fanout_parallelism_cap_100() {
        // Request 150 workers — verify cap applied (no panic, runs to completion).
        let svc = Arc::new(StubTaskService::new(vec![]));
        let workers: Vec<String> = (0..150).map(|i| format!("w{i}")).collect();
        let config = FanOutConfig {
            workers,
            aggregator: "agg".into(),
            partition_fn: None,
        };
        let pattern = FanOutFanInPattern::new(config);
        let ctx = make_ctx(svc.clone());

        let result = pattern.execute(json!({}), ctx).await.unwrap();
        // 150 workers + 1 aggregator = 151 calls.
        assert_eq!(svc.call_count.load(Ordering::SeqCst), 151);
        assert!(result.get("agent").is_some());
    }
}
