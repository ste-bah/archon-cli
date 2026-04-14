//! Integration smoke tests for `run_event_loop` (TUI-106).
//!
//! These tests exercise the non-blocking event loop end-to-end against
//! an in-memory `MockRunner` / `SlowRunner`, verifying the three
//! TUI-106 spec requirements:
//!
//! 1. A `TuiEvent::UserInput` spawns a turn against the configured
//!    runner.
//! 2. A `TuiEvent::SlashCancel` aborts a long-running turn well before
//!    it would complete on its own (wall-clock budget assertion).
//! 3. Multiple queued prompts drain FIFO through the dispatcher.
//!
//! The tests use `tokio::test(flavor = "multi_thread", worker_threads = 2)`
//! so `run_event_loop` can run concurrently with the test body via
//! `tokio::spawn`. Real `tokio::time::sleep` is used for waits (no
//! `pause()` / `advance()`).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_core::agent::AgentEvent;
use archon_tui::{AgentRouter, EventLoopConfig, TurnRunner, app::TuiEvent, run_event_loop};
use tokio::sync::mpsc::unbounded_channel;

/// Records every prompt `run_turn` is called with, in call order.
struct MockRunner {
    log: Arc<Mutex<Vec<String>>>,
}

impl TurnRunner for MockRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let log = self.log.clone();
        Box::pin(async move {
            log.lock().unwrap().push(prompt);
            Ok(())
        })
    }
}

/// Runner that sleeps for 10s before returning — gives `cancel_current`
/// an `.await` point to land on so the abort actually exercises the
/// cancel-safe join handle path.
struct SlowRunner;

impl TurnRunner for SlowRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        })
    }
}

/// Router that ignores every `switch` call. We don't test switch_agent
/// behaviour here (covered in TUI-104) — just that the event loop
/// threads the call through without panicking.
struct NoopRouter;

impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_event_loop_user_input_spawns_turn() {
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(MockRunner { log: log.clone() });
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);

    let (tui_event_tx, tui_event_rx) = unbounded_channel::<TuiEvent>();
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<AgentEvent>();

    let cfg = EventLoopConfig {
        tui_event_rx,
        agent_event_tx,
        runner,
        router,
    };

    let handle = tokio::spawn(async move { run_event_loop(cfg).await });

    tui_event_tx
        .send(TuiEvent::UserInput("hello".into()))
        .expect("send UserInput");

    // Let the select loop pick up the event, spawn the turn, run it,
    // and drain completion.
    tokio::time::sleep(Duration::from_millis(150)).await;

    tui_event_tx.send(TuiEvent::Done).expect("send Done");

    handle
        .await
        .expect("join run_event_loop")
        .expect("run_event_loop Ok");

    let recorded = log.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec!["hello".to_string()],
        "MockRunner should have recorded exactly one prompt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_event_loop_cancel_aborts_running_turn() {
    let runner: Arc<dyn TurnRunner> = Arc::new(SlowRunner);
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);

    let (tui_event_tx, tui_event_rx) = unbounded_channel::<TuiEvent>();
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<AgentEvent>();

    let cfg = EventLoopConfig {
        tui_event_rx,
        agent_event_tx,
        runner,
        router,
    };

    let start = Instant::now();

    let handle = tokio::spawn(async move { run_event_loop(cfg).await });

    tui_event_tx
        .send(TuiEvent::UserInput("slow".into()))
        .expect("send UserInput");

    // Give the turn time to start and hit the 10s sleep.
    tokio::time::sleep(Duration::from_millis(75)).await;

    tui_event_tx
        .send(TuiEvent::SlashCancel)
        .expect("send SlashCancel");

    // Give the dispatcher a beat to process the abort.
    tokio::time::sleep(Duration::from_millis(150)).await;

    tui_event_tx.send(TuiEvent::Done).expect("send Done");

    handle
        .await
        .expect("join run_event_loop")
        .expect("run_event_loop Ok");

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "cancel should have aborted the 10s sleep; elapsed={:?}",
        elapsed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_event_loop_drains_queue_after_completion() {
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(MockRunner { log: log.clone() });
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);

    let (tui_event_tx, tui_event_rx) = unbounded_channel::<TuiEvent>();
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<AgentEvent>();

    let cfg = EventLoopConfig {
        tui_event_rx,
        agent_event_tx,
        runner,
        router,
    };

    let handle = tokio::spawn(async move { run_event_loop(cfg).await });

    tui_event_tx
        .send(TuiEvent::UserInput("p1".into()))
        .expect("send p1");
    tui_event_tx
        .send(TuiEvent::UserInput("p2".into()))
        .expect("send p2");
    tui_event_tx
        .send(TuiEvent::UserInput("p3".into()))
        .expect("send p3");

    // Enough wall-clock for all 3 to spawn, run, complete, and drain
    // through pending_queue in FIFO order.
    tokio::time::sleep(Duration::from_millis(400)).await;

    tui_event_tx.send(TuiEvent::Done).expect("send Done");

    handle
        .await
        .expect("join run_event_loop")
        .expect("run_event_loop Ok");

    let recorded = log.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec!["p1".to_string(), "p2".to_string(), "p3".to_string()],
        "pending queue should drain FIFO"
    );
}
