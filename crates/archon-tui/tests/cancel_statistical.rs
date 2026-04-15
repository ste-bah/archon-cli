//! TUI-110: Statistical cancellation latency test — 200 trials, 99.5% success budget.
//!
//! #[ignore]-gated; runs via `cargo test -j1 -p archon-tui --test cancel_statistical -- --ignored --test-threads=2`.
//!
//! Deviations from spec:
//!
//! D1. Spec says "MockAgent". No Agent trait exists in archon-tui. Fake is FakeTurnRunner
//!     implementing the TurnRunner trait (matches TUI-100 deviation carried through 101..104).
//!
//! D2. YieldGate omitted from the fake body. YieldGate::tick applies to sync-heavy inner
//!     loops needing cooperative cancellation injection. The fake is fully async —
//!     tokio::time::sleep(1ms).await is itself a .await point, so the tokio runtime can
//!     cancel the future on abort without YieldGate assistance.
//!
//! D3. Spec's observation mechanism ("JoinHandle::is_finished post-abort") is impossible:
//!     AgentDispatcher::cancel_current() takes the handle via self.current_query.take(),
//!     so after the call current_query = None and poll_completion returns None forever.
//!     CancelOutcome::Aborted.elapsed_ms measures two adjacent instructions (<1µs) and is
//!     worthless as a latency signal. Replacement: DropSentinel pattern — an owned
//!     sentinel struct holding Arc<AtomicBool> is bound inside the fake's async block
//!     before the .await loop; tokio::abort() drops the future, Drop impl flips the flag,
//!     the test polls the flag with 1ms tick + 500ms timeout and measures
//!     cancel_sent_at.elapsed() at flag-flip. This proves the future's owned state
//!     actually released on abort — strictly stronger than any JoinHandle status check.
//!
//! D4. Jitter PRNG is an inline LCG seeded with 42. Workspace has rand 0.9 but adding
//!     rand to archon-tui dev-deps would edit Cargo.toml (frozen). Seed 42 is documented
//!     for reproducibility.
//!
//! D5. worker_threads = 4 is net-new to archon-tui tests (prior tests used 2). The 4-way
//!     runtime parallelism lets the test-driver task, the spawned turn, the poll loop,
//!     and the timeout watchdog all progress concurrently without starvation.

struct NoopRouter;
impl archon_tui::AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct DropSentinel {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for DropSentinel {
    fn drop(&mut self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct FakeTurnRunner {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl archon_tui::TurnRunner for FakeTurnRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let flag = self.flag.clone();
        Box::pin(async move {
            // YieldGate omitted intentionally (D2): tokio::time::sleep(1ms).await already
            // satisfies cooperative cancellation. YieldGate::tick applies to sync-heavy
            // inner loops, not fully-async fakes.
            let _sentinel = DropSentinel { flag };
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
    }
}

/// Inline LCG (Numerical Recipes constants) — avoids adding `rand` to dev-deps,
/// which would edit Cargo.toml (frozen).
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn gen_range_1_to_100(&mut self) -> u64 {
        (self.next() % 100) + 1 // 1..=100 ms
    }
}

fn make_dispatcher() -> archon_tui::AgentDispatcher {
    let (tx, _rx) =
        tokio::sync::mpsc::unbounded_channel::<archon_core::agent::TimestampedEvent>();
    let router: std::sync::Arc<dyn archon_tui::AgentRouter> = std::sync::Arc::new(NoopRouter);
    archon_tui::AgentDispatcher::new(router, tx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn test_cancel_200_trials_99_5_percent_success() {
    const TRIALS: usize = 200;
    const SUCCESS_THRESHOLD: usize = 199; // 99.5%
    const P99_BUDGET_MS: u64 = 500;
    const P50_BUDGET_MS: u64 = 50;

    let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut lcg = Lcg::new(42);
        let mut latencies_ms: Vec<u64> = Vec::with_capacity(TRIALS);
        let mut success_count = 0usize;

        for trial in 0..TRIALS {
            let mut dispatcher = make_dispatcher();
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let runner: std::sync::Arc<dyn archon_tui::TurnRunner> =
                std::sync::Arc::new(FakeTurnRunner { flag: flag.clone() });

            let dispatch_result =
                dispatcher.spawn_turn(format!("trial-{trial}"), runner.clone());
            // Must be Running — no prior query.
            match dispatch_result {
                archon_tui::DispatchResult::Running { .. } => {}
                _ => panic!("trial {trial}: spawn_turn did not return Running"),
            }

            // Jitter 1..=100 ms to let the fake's sleep loop actually enter .await
            let jitter_ms = lcg.gen_range_1_to_100();
            tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;

            let cancel_sent_at = std::time::Instant::now();
            let _ = dispatcher.cancel_current();

            // Poll DropSentinel flag with 1ms tick, 500ms deadline
            let mut observed_latency_ms: Option<u64> = None;
            let deadline = cancel_sent_at + std::time::Duration::from_millis(P99_BUDGET_MS);
            loop {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    observed_latency_ms =
                        Some(cancel_sent_at.elapsed().as_millis() as u64);
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }

            match observed_latency_ms {
                Some(ms) => {
                    success_count += 1;
                    latencies_ms.push(ms);
                }
                None => {
                    // Timed out — record as 500 (the budget) so p99 still sees it
                    latencies_ms.push(P99_BUDGET_MS);
                }
            }

            // Per-trial clean-state invariant
            assert!(
                dispatcher.current_query.is_none(),
                "trial {trial}: current_query not None after cancel_current"
            );
            assert!(
                dispatcher.pending_queue.is_empty(),
                "trial {trial}: pending_queue not empty after cancel_current"
            );
        }

        latencies_ms.sort();
        let p50 = latencies_ms[99]; // index 99 of 200 sorted
        let p95 = latencies_ms[189]; // index 189
        let p99 = latencies_ms[197]; // index 197
        let tail = &latencies_ms[190..];

        if success_count < SUCCESS_THRESHOLD || p99 > P99_BUDGET_MS || p50 > P50_BUDGET_MS {
            panic!(
                "histogram: trials={TRIALS} successes={success_count} failures={} p50={p50} p95={p95} p99={p99} tail={tail:?}",
                TRIALS - success_count
            );
        }

        println!(
            "histogram: trials={TRIALS} successes={success_count} p50={p50} p95={p95} p99={p99} tail={tail:?}"
        );
    })
    .await;

    result.expect("200-trial test exceeded 120s safety timeout");
}
