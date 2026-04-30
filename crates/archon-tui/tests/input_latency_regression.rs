//! Input latency regression test for TASK-TUI-109 / NFR-TUI-PERF-002.
//!
//! Directly targets `AgentDispatcher::spawn_turn` — not the full event
//! loop — to isolate the dispatcher-side guarantee that spawning a
//! turn is O(1) even while another turn is in flight. This is the
//! core invariant behind "input never blocks": if `spawn_turn` is
//! non-blocking and O(1), the event loop cannot starve on keystrokes
//! regardless of how long a running turn takes.
//!
//! Scenario:
//!   1. Construct a `SlowRunner` whose `run_turn` sleeps for 5
//!      seconds. This guarantees the first spawned turn is still
//!      running for the entire duration of the test.
//!   2. Call `spawn_turn` once — asserted to return `Running` and
//!      complete in well under 100ms.
//!   3. Fire 100 additional `spawn_turn` calls back-to-back,
//!      simulating a user hammering keystrokes while the agent is
//!      busy. Each call is timed; each call's elapsed time must be
//!      under 100ms (the hard NFR budget). We also compute the 99th
//!      percentile and assert a much tighter <10ms headroom to
//!      catch regressions before they reach the hard budget.
//!   4. Every rapid call must return `DispatchResult::Queued` — the
//!      slow turn is still occupying the Running slot.
//!
//! We deliberately do NOT `.await` the slow turn. The test function
//! ends, the dispatcher drops, and the tokio JoinHandle detaches —
//! the tokio test runtime shuts down on return, so leaking the
//! 5-second sleep task is harmless.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::TimestampedEvent;
use archon_tui::{AgentDispatcher, AgentRouter, DispatchResult, TurnRunner};
use tokio::sync::mpsc;

/// Runner whose `run_turn` sleeps 5s before returning. Long enough
/// that it will still be in flight after we fire all 100 follow-up
/// spawn_turn calls, guaranteeing we exercise the
/// "current_query is Some -> push onto pending_queue" path.
struct SlowRunner;

impl TurnRunner for SlowRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        })
    }
}

/// Router that ignores every switch call — this test does not
/// exercise agent switching, only dispatch latency.
struct NoopRouter;

impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// `DispatchResult` does not implement `Debug` in the public API.
/// Tiny helper so failure assertions produce a readable diagnostic.
fn dispatch_variant(r: &DispatchResult) -> &'static str {
    match r {
        DispatchResult::Queued => "Queued",
        DispatchResult::Running { .. } => "Running",
        DispatchResult::Rejected(_) => "Rejected",
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_input_dispatch_latency_during_running_turn_under_100ms() {
    let (agent_event_tx, _agent_event_rx) = mpsc::unbounded_channel::<TimestampedEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let runner: Arc<dyn TurnRunner> = Arc::new(SlowRunner);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    // First spawn — starts the 5s slow turn. This should be near-
    // instant because spawn_turn just constructs a JoinHandle via
    // `tokio::spawn` and stores it in `current_query`.
    let t0 = Instant::now();
    let first = dispatcher.spawn_turn("long-running prompt".to_string(), runner.clone());
    let first_elapsed = t0.elapsed();
    assert!(
        first_elapsed < Duration::from_millis(100),
        "first spawn_turn took {}ms, expected <100ms — dispatcher is blocking",
        first_elapsed.as_millis()
    );
    assert!(
        matches!(first, DispatchResult::Running { .. }),
        "expected first spawn_turn to return Running, got {}",
        dispatch_variant(&first),
    );

    // 100 rapid keystrokes — simulated by firing spawn_turn 100
    // times. The slow turn from the first call is still running, so
    // every one of these must land in the pending_queue and return
    // DispatchResult::Queued.
    let mut samples: Vec<Duration> = Vec::with_capacity(100);
    for i in 0..100 {
        let t = Instant::now();
        let result = dispatcher.spawn_turn(format!("k{}", i), runner.clone());
        let elapsed = t.elapsed();
        samples.push(elapsed);

        assert!(
            elapsed < Duration::from_millis(100),
            "spawn_turn #{} took {}ms, exceeded 100ms NFR-TUI-PERF-002 budget",
            i,
            elapsed.as_millis()
        );
        assert!(
            matches!(result, DispatchResult::Queued),
            "spawn_turn #{} returned {}, expected Queued (slow turn still in flight)",
            i,
            dispatch_variant(&result)
        );
    }

    // Sanity-check that the pending queue actually holds all 100
    // entries — proves the test scenario is meaningful (we didn't
    // silently drop anything or accidentally drain).
    assert_eq!(
        dispatcher.pending_queue.len(),
        100,
        "expected 100 entries in pending_queue, got {}",
        dispatcher.pending_queue.len()
    );
    assert!(
        dispatcher.current_query.is_some(),
        "expected current_query to still be Some (slow turn in flight)"
    );

    // Tight p99 headroom check. samples.len() == 100, sorted
    // ascending, so samples[98] is the 99th percentile. We assert
    // <10ms, which is an order of magnitude below the hard 100ms
    // budget — catches perf regressions early.
    samples.sort();
    let p99 = samples[98];
    assert!(
        p99 < Duration::from_millis(10),
        "p99 = {}ms, expected <10ms headroom. samples (sorted): {:?}",
        p99.as_millis(),
        samples
    );

    // NOTE: we intentionally do NOT await the slow turn. The test
    // runtime shuts down on function return; the detached 5s sleep
    // task gets dropped harmlessly.
}
