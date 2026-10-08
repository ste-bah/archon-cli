//! Input latency regression test for TASK-TUI-109 / NFR-TUI-PERF-002.
//!
//! Directly targets `AgentDispatcher::spawn_turn` — not the full event
//! loop — to isolate the dispatcher-side guarantee that spawning a
//! turn never blocks on the turn itself, even while another turn is in
//! flight. This is the core invariant behind "input never blocks": if
//! `spawn_turn` returns without waiting for the turn, the event loop
//! cannot starve on keystrokes regardless of how long a turn takes.
//!
//! The property is proven STRUCTURALLY, not with a cold wall-clock
//! bound (issue #365: a single cold `spawn_turn` took 219 ms on a
//! loaded CI runner, which is scheduling noise, not a block):
//!
//!   1. `GatedRunner::run_turn` cannot finish until the test opens a
//!      gate (`watch` channel). The gate stays closed for the whole
//!      dispatch phase.
//!   2. The dispatch phase (1 + 100 `spawn_turn` calls) runs on its own
//!      OS thread. The test waits for it with a generous timeout. If
//!      `spawn_turn` waited for the turn, the thread could never return
//!      while the gate is closed, so the timeout fires and the test
//!      FAILS (the gate is then opened so the stuck thread can exit).
//!   3. The first call must return `Running`; the 100 rapid calls must
//!      return `Queued`; the turn must have started and must not have
//!      finished (gate still closed).
//!   4. The gate is opened and the turn must then complete — proves
//!      the gate really was the only thing holding the turn.
//!
//! Timing is kept only as a robust headroom check: the MEDIAN of the
//! 100 warm `Queued` calls must stay under 10 ms. One cold call is
//! never bounded.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::TimestampedEvent;
use archon_tui::{AgentDispatcher, AgentRouter, DispatchResult, TurnRunner};
use tokio::sync::{mpsc, oneshot, watch};

/// Generous upper bound for the dispatch phase. A non-blocking
/// dispatcher finishes 101 calls in microseconds; this only guards
/// against a dispatcher that waits on the (gated, never-ending) turn.
const DISPATCH_PHASE_TIMEOUT: Duration = Duration::from_secs(30);

/// Runner whose `run_turn` cannot complete until the test opens the
/// gate. Counts how many turns started and finished so the test can
/// prove the turn was in flight (started, not finished) while every
/// `spawn_turn` call returned.
struct GatedRunner {
    gate: watch::Receiver<bool>,
    started: AtomicUsize,
    finished: AtomicUsize,
}

impl TurnRunner for GatedRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let mut gate = self.gate.clone();
        Box::pin(async move {
            self.started.fetch_add(1, Ordering::SeqCst);
            gate.wait_for(|open| *open)
                .await
                .map_err(|_| anyhow::anyhow!("gate sender dropped"))?;
            self.finished.fetch_add(1, Ordering::SeqCst);
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

/// Output of the dispatch phase, sent back from the dispatch thread.
struct DispatchPhase {
    dispatcher: AgentDispatcher,
    first: DispatchResult,
    rapid: Vec<(DispatchResult, Duration)>,
}

/// Poll `cond` until it is true or `limit` elapses.
async fn wait_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cond()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_input_dispatch_latency_during_running_turn_under_100ms() {
    let (gate_tx, gate_rx) = watch::channel(false);
    let runner = Arc::new(GatedRunner {
        gate: gate_rx,
        started: AtomicUsize::new(0),
        finished: AtomicUsize::new(0),
    });
    let (agent_event_tx, _agent_event_rx) =
        mpsc::channel::<TimestampedEvent>(archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY);
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    // Dispatch phase on a plain OS thread (runtime context entered so
    // `tokio::spawn` inside the dispatcher works). A blocking
    // `spawn_turn` then stalls only this thread, never the test.
    let rt = tokio::runtime::Handle::current();
    let dyn_runner: Arc<dyn TurnRunner> = runner.clone();
    let (done_tx, done_rx) = oneshot::channel::<DispatchPhase>();
    std::thread::spawn(move || {
        let _guard = rt.enter();
        let first = dispatcher.spawn_turn("long-running prompt".to_string(), dyn_runner.clone());
        let mut rapid = Vec::with_capacity(100);
        for i in 0..100 {
            let t = Instant::now();
            let result = dispatcher.spawn_turn(format!("k{}", i), dyn_runner.clone());
            rapid.push((result, t.elapsed()));
        }
        let _ = done_tx.send(DispatchPhase {
            dispatcher,
            first,
            rapid,
        });
    });

    let phase = match tokio::time::timeout(DISPATCH_PHASE_TIMEOUT, done_rx).await {
        Ok(Ok(phase)) => phase,
        Ok(Err(_)) => panic!("dispatch thread panicked before reporting results"),
        Err(_) => {
            // Let the stuck thread finish so it does not leak forever.
            let _ = gate_tx.send(true);
            panic!(
                "spawn_turn did not return within {:?} while the running turn's gate \
                 was closed — the dispatcher is blocking on the in-flight turn",
                DISPATCH_PHASE_TIMEOUT
            );
        }
    };
    let DispatchPhase {
        mut dispatcher,
        first,
        rapid,
    } = phase;

    // Every call returned while the gate was still closed.
    assert!(
        !*gate_tx.borrow(),
        "gate must still be closed after dispatch"
    );
    assert!(
        matches!(first, DispatchResult::Running { .. }),
        "expected first spawn_turn to return Running, got {}",
        dispatch_variant(&first),
    );
    for (i, (result, _)) in rapid.iter().enumerate() {
        assert!(
            matches!(result, DispatchResult::Queued),
            "spawn_turn #{} returned {}, expected Queued (gated turn still in flight)",
            i,
            dispatch_variant(result)
        );
    }

    // The turn really is in flight: it started, and it cannot have
    // finished because the gate is closed.
    assert!(
        wait_until(DISPATCH_PHASE_TIMEOUT, || runner
            .started
            .load(Ordering::SeqCst)
            == 1)
        .await,
        "gated turn never started (started = {})",
        runner.started.load(Ordering::SeqCst)
    );
    assert_eq!(
        runner.finished.load(Ordering::SeqCst),
        0,
        "gated turn finished while the gate was closed"
    );
    assert_eq!(
        dispatcher.pending_queue.len(),
        100,
        "expected 100 entries in pending_queue, got {}",
        dispatcher.pending_queue.len()
    );
    let in_flight_unfinished = dispatcher
        .current_query
        .as_ref()
        .is_some_and(|h| !h.is_finished());
    assert!(
        in_flight_unfinished,
        "expected current_query to hold the unfinished gated turn"
    );

    // Robust headroom check on WARM calls only: the median of the 100
    // Queued calls. Immune to one preempted or cold sample.
    let mut samples: Vec<Duration> = rapid.iter().map(|(_, d)| *d).collect();
    samples.sort();
    let median = samples[samples.len() / 2];
    assert!(
        median < Duration::from_millis(10),
        "median Queued spawn_turn = {:?}, expected <10ms. samples (sorted): {:?}",
        median,
        samples
    );

    // Open the gate: the turn must now complete, proving the gate was
    // the only thing holding it.
    gate_tx.send(true).expect("gate receiver alive");
    assert!(
        wait_until(DISPATCH_PHASE_TIMEOUT, || runner
            .finished
            .load(Ordering::SeqCst)
            >= 1)
        .await,
        "gated turn did not finish after the gate was opened"
    );
    let finished_handle = dispatcher
        .current_query
        .take()
        .expect("current_query still set (no poll_completion was called)");
    finished_handle
        .await
        .expect("turn task joined")
        .expect("turn returned Ok");
}
