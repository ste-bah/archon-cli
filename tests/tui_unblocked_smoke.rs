//! TASK-TUI-107 regression smoke test.
//!
//! Proves that `AgentDispatcher::spawn_turn` does NOT block on a prior
//! in-flight turn. The failure mode this test guards against is the
//! pre-TUI-107 pattern in `src/main.rs` where every dispatch did
//! `handle.await` on the previous turn's JoinHandle before spawning the
//! next one — serializing the input loop.
//!
//! Shape:
//! 1. Spawn turn "first" against a FakeTurnRunner with a 500ms delay.
//!    `spawn_turn` must return DispatchResult::Running in <10ms.
//! 2. Immediately spawn turn "second". It must return
//!    DispatchResult::Queued in <10ms — the dispatcher must NOT wait
//!    for "first" to finish.
//! 3. Drive `poll_completion` on a 16ms tick until both turns drain.
//! 4. Assert FIFO order: runner's log == ["first", "second"].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::agent::{AgentEvent, TimestampedEvent};
use archon_tui::{AgentDispatcher, AgentRouter, DispatchResult, TurnOutcome, TurnRunner};
use tokio::sync::Mutex;
use tokio::sync::mpsc::unbounded_channel;

/// Fake TurnRunner that sleeps for `delay`, then appends the prompt to a
/// shared log. Used to simulate a long-running agent turn without pulling
/// in the real `archon_core::agent::Agent`.
struct FakeTurnRunner {
    delay: Duration,
    log: Arc<Mutex<Vec<String>>>,
}

impl TurnRunner for FakeTurnRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let delay = self.delay;
        let log = self.log.clone();
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            log.lock().await.push(prompt);
            Ok(())
        })
    }
}

struct NoopRouter;

impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Tolerance for "spawn_turn is non-blocking." The real call returns in
/// microseconds; 50ms gives plenty of slack for a cold tokio worker in
/// CI without being so loose that a regression to serialized
/// `handle.await` would slip through (serialized path would take ~500ms).
const NON_BLOCKING_TOLERANCE: Duration = Duration::from_millis(50);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dispatcher_does_not_block_on_inflight_turn() {
    // Agent event channel — unused in this test, but AgentDispatcher::new
    // requires a live UnboundedSender<AgentEvent>.
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();

    // Fake runner with a 500ms per-turn delay. If the dispatcher
    // serialized, spawning the second turn would wait at least 500ms for
    // the first to complete — failing the NON_BLOCKING_TOLERANCE assert
    // below.
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(FakeTurnRunner {
        delay: Duration::from_millis(500),
        log: log.clone(),
    });

    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    // Dispatch #1 — expect Running almost instantly.
    let t0 = Instant::now();
    let result1 = dispatcher.spawn_turn("first".to_string(), runner.clone());
    let elapsed1 = t0.elapsed();
    assert!(
        elapsed1 < NON_BLOCKING_TOLERANCE,
        "spawn_turn #1 took {elapsed1:?}, expected < {NON_BLOCKING_TOLERANCE:?}"
    );
    match result1 {
        DispatchResult::Running { .. } => {}
        DispatchResult::Queued => panic!("first dispatch should be Running, got Queued"),
        DispatchResult::Rejected(err) => panic!("first dispatch rejected: {err}"),
    }

    // Dispatch #2 — expect Queued almost instantly. This is the smoking
    // gun: pre-TUI-107 this would have waited for the 500ms turn.
    let t1 = Instant::now();
    let result2 = dispatcher.spawn_turn("second".to_string(), runner.clone());
    let elapsed2 = t1.elapsed();
    assert!(
        elapsed2 < NON_BLOCKING_TOLERANCE,
        "spawn_turn #2 took {elapsed2:?}, expected < {NON_BLOCKING_TOLERANCE:?} \
         (serialized regression?)"
    );
    match result2 {
        DispatchResult::Queued => {}
        DispatchResult::Running { .. } => {
            panic!("second dispatch should be Queued, got Running")
        }
        DispatchResult::Rejected(err) => panic!("second dispatch rejected: {err}"),
    }

    // Drive the poll loop until both turns have drained. 2s safety timeout
    // (the turns take ~500ms each sequentially = ~1s total; 2s is 2x
    // slack).
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut outcomes: Vec<String> = Vec::new();
    while outcomes.len() < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(16)).await;
        if let Some(outcome) = dispatcher.poll_completion() {
            match outcome {
                TurnOutcome::Completed => outcomes.push("completed".to_string()),
                TurnOutcome::Cancelled => outcomes.push("cancelled".to_string()),
                TurnOutcome::Failed(err) => panic!("unexpected turn failure: {err}"),
            }
        }
    }
    assert_eq!(
        outcomes.len(),
        2,
        "expected two TurnOutcome::Completed observations, got {}: {outcomes:?}",
        outcomes.len()
    );
    assert_eq!(outcomes[0], "completed");
    assert_eq!(outcomes[1], "completed");

    // Assert FIFO order preserved: "first" ran before "second".
    let log = log.lock().await;
    assert_eq!(
        &*log,
        &["first".to_string(), "second".to_string()],
        "FIFO order not preserved"
    );
}
