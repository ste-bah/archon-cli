//! Unit tests for [`crate::task_dispatch`]. Split from `task_dispatch.rs`
//! via `#[cfg(test)] #[path = ...] mod tests;` to keep the implementation
//! file under its <300 line budget (see TASK-TUI-100 scaffold header).
//!
//! This file is included as a nested `mod tests` from within
//! `task_dispatch.rs`, so `super::*` resolves to items in `task_dispatch`.

use super::*;
use archon_core::agent::AgentEvent;
use std::sync::Arc;

#[test]
fn queued_prompt_fields_are_reachable() {
    let runner: Arc<dyn TurnRunner> = Arc::new(NoopRunner);
    let q = QueuedPrompt {
        prompt: "hello".into(),
        agent_id: None,
        submitted_at: std::time::Instant::now(),
        runner,
    };
    assert_eq!(q.prompt, "hello");
    assert!(q.agent_id.is_none());
    let _ = q.submitted_at;
    let _ = q.runner;
}

#[test]
fn cancel_outcome_variants_are_exhaustive() {
    let a = CancelOutcome::NoInflight;
    let b = CancelOutcome::Aborted { elapsed_ms: 0 };
    for v in [a, b] {
        match v {
            CancelOutcome::NoInflight => {}
            CancelOutcome::Aborted { elapsed_ms: _ } => {}
        }
    }
}

#[test]
fn dispatch_result_variants_are_exhaustive() {
    let r1 = DispatchResult::Queued;
    let r2 = DispatchResult::Running { spawned_at: std::time::Instant::now() };
    let r3 = DispatchResult::Rejected("nope".into());
    for v in [r1, r2, r3] {
        match v {
            DispatchResult::Queued => {}
            DispatchResult::Running { spawned_at: _ } => {}
            DispatchResult::Rejected(_) => {}
        }
    }
}

#[test]
fn turn_outcome_variants_are_exhaustive() {
    let a = TurnOutcome::Completed;
    let b = TurnOutcome::Cancelled;
    let c = TurnOutcome::Failed("boom".into());
    for v in [a, b, c] {
        match v {
            TurnOutcome::Completed => {}
            TurnOutcome::Cancelled => {}
            TurnOutcome::Failed(_) => {}
        }
    }
}

struct NoopRouter;
impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn agent_router_is_object_safe_and_buildable() {
    let r: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    r.switch("foo").unwrap();
}

struct NoopRunner;
impl TurnRunner for NoopRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn turn_runner_is_object_safe_and_buildable() {
    let _: Arc<dyn TurnRunner> = Arc::new(NoopRunner);
}

#[test]
fn dispatcher_constructs_with_noop_router() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let d = AgentDispatcher::new(router, tx);
    assert_eq!(d.queue_len(), 0);
    assert!(!d.is_busy());
}

// ---- TASK-TUI-101 tests ----

/// Configurable fake [`TurnRunner`] used by TASK-TUI-101 tests. Sleeps for
/// `sleep_ms` inside the spawned future then returns `Ok(())`.
struct MockTurnRunner {
    sleep_ms: u64,
}

impl TurnRunner for MockTurnRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let ms = self.sleep_ms;
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            Ok(())
        })
    }
}

fn make_dispatcher() -> AgentDispatcher {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    AgentDispatcher::new(router, tx)
}

#[tokio::test]
async fn test_spawn_turn_when_idle_returns_running() {
    let mut d = make_dispatcher();
    let runner: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 10 });
    let result = d.spawn_turn("hello".into(), runner);
    assert!(matches!(result, DispatchResult::Running { .. }));
    assert!(d.is_busy());
    assert_eq!(d.queue_len(), 0);
    if let Some(h) = d.current_query.take() {
        let _ = h.await;
    }
}

#[tokio::test]
async fn test_spawn_turn_when_busy_queues() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 500 });
    let _ = d.spawn_turn("first".into(), slow.clone());
    let result = d.spawn_turn("second".into(), slow);
    assert!(matches!(result, DispatchResult::Queued));
    assert_eq!(d.queue_len(), 1);
    if let Some(h) = d.current_query.take() {
        let _ = h.await;
    }
}

#[tokio::test]
async fn test_spawn_turn_does_not_await_agent() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 500 });
    let start = std::time::Instant::now();
    let _ = d.spawn_turn("hello".into(), slow);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 5,
        "spawn_turn blocked for {}ms (>=5ms)",
        elapsed.as_millis()
    );
    if let Some(h) = d.current_query.take() {
        let _ = h.await;
    }
}

// ---- TASK-TUI-102 tests ----

#[tokio::test]
async fn test_cancel_when_no_inflight_returns_noinflight() {
    let mut d = make_dispatcher();
    let outcome = d.cancel_current();
    assert!(matches!(outcome, CancelOutcome::NoInflight));
    assert!(!d.is_busy());
}

#[tokio::test]
async fn test_cancel_aborts_running_turn() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 10_000 });
    let _ = d.spawn_turn("hello".into(), slow);
    assert!(d.is_busy());
    let outcome = d.cancel_current();
    assert!(
        matches!(outcome, CancelOutcome::Aborted { elapsed_ms } if elapsed_ms < 50),
        "expected Aborted with elapsed_ms < 50",
    );
    assert!(!d.is_busy());
    assert!(d.current_query.is_none());
}

#[tokio::test]
async fn test_cancel_does_not_await_handle() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 10_000 });
    let _ = d.spawn_turn("hello".into(), slow);
    let start = std::time::Instant::now();
    let _ = d.cancel_current();
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(10),
        "cancel_current blocked for {}ms (>=10ms) — likely awaited the handle",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn test_cancel_twice_is_idempotent() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 10_000 });
    let _ = d.spawn_turn("hello".into(), slow);
    let first = d.cancel_current();
    assert!(matches!(first, CancelOutcome::Aborted { .. }));
    let second = d.cancel_current();
    assert!(matches!(second, CancelOutcome::NoInflight));
    assert!(!d.is_busy());
}

// ---- TASK-TUI-103 tests ----

/// Outcome a [`ConfigurableRunner`] should produce for a single run_turn call.
#[derive(Clone)]
enum MockOutcome {
    /// Return `Ok(())` after an optional tiny sleep.
    Success,
    /// Return `Err(anyhow!(msg))` after an optional tiny sleep.
    Error(String),
    /// Panic with the given message inside the future body.
    Panic(String),
    /// Sleep for an essentially unbounded duration — used for cancel tests.
    SleepForever,
}

/// Fake runner for TASK-TUI-103 that can express every [`TurnOutcome`]
/// branch and records the FIFO order of prompts it actually ran.
struct ConfigurableRunner {
    outcomes: std::sync::Mutex<std::collections::VecDeque<MockOutcome>>,
    recorded: Arc<std::sync::Mutex<Vec<String>>>,
    run_delay_ms: u64,
}

impl ConfigurableRunner {
    fn new(
        outcomes: Vec<MockOutcome>,
        recorded: Arc<std::sync::Mutex<Vec<String>>>,
        run_delay_ms: u64,
    ) -> Self {
        Self {
            outcomes: std::sync::Mutex::new(outcomes.into()),
            recorded,
            run_delay_ms,
        }
    }
}

impl TurnRunner for ConfigurableRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(MockOutcome::Success);
        let recorded = Arc::clone(&self.recorded);
        let delay = self.run_delay_ms;
        Box::pin(async move {
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            recorded.lock().unwrap().push(prompt);
            match outcome {
                MockOutcome::Success => Ok(()),
                MockOutcome::Error(msg) => Err(anyhow::anyhow!(msg)),
                MockOutcome::Panic(msg) => panic!("{msg}"),
                MockOutcome::SleepForever => {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    Ok(())
                }
            }
        })
    }
}

/// Event-loop helper: drain the dispatcher to idle, collecting every
/// [`TurnOutcome`] the drain path emits. Bounded deadline so test failures
/// don't hang.
async fn drain_until_idle(d: &mut AgentDispatcher) -> Vec<TurnOutcome> {
    let mut outcomes = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while (d.is_busy() || d.queue_len() > 0) && std::time::Instant::now() < deadline {
        if let Some(outcome) = d.poll_completion() {
            outcomes.push(outcome);
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }
    while let Some(outcome) = d.poll_completion() {
        outcomes.push(outcome);
    }
    outcomes
}

#[tokio::test]
async fn test_poll_completion_idle_returns_none() {
    let mut d = make_dispatcher();
    assert!(d.poll_completion().is_none());
    assert!(!d.is_busy());
    assert_eq!(d.queue_len(), 0);
}

#[tokio::test]
async fn test_poll_completion_running_returns_none() {
    let mut d = make_dispatcher();
    let slow: Arc<dyn TurnRunner> = Arc::new(MockTurnRunner { sleep_ms: 10_000 });
    let _ = d.spawn_turn("hello".into(), slow);
    // Handle is not finished — poll must return None without draining.
    assert!(d.poll_completion().is_none());
    assert!(d.is_busy());
    if let Some(h) = d.current_query.take() {
        h.abort();
        let _ = h.await;
    }
}

#[tokio::test]
async fn test_poll_completion_success_branch() {
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        vec![MockOutcome::Success],
        Arc::clone(&recorded),
        1,
    ));
    let _ = d.spawn_turn("p1".into(), runner);
    let outcomes = drain_until_idle(&mut d).await;
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], TurnOutcome::Completed));
    assert!(!d.is_busy());
    assert_eq!(d.queue_len(), 0);
    assert_eq!(recorded.lock().unwrap().as_slice(), &["p1".to_string()]);
}

#[tokio::test]
async fn test_poll_completion_failed_branch() {
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        vec![MockOutcome::Error("boom".into())],
        Arc::clone(&recorded),
        1,
    ));
    let _ = d.spawn_turn("p1".into(), runner);
    let outcomes = drain_until_idle(&mut d).await;
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        TurnOutcome::Failed(msg) => assert!(
            msg.contains("boom"),
            "expected failure message to contain 'boom', got: {msg}"
        ),
        other => panic!("expected Failed, got {:?}", std::mem::discriminant(other)),
    }
    assert!(!d.is_busy());
}

#[tokio::test]
async fn test_poll_completion_cancelled_branch() {
    // The drain path observes `TurnOutcome::Cancelled` when a tokio task is
    // aborted *without* going through `cancel_current()` (which `take`s the
    // handle). We simulate that by spawning a runner that sleeps forever,
    // manually aborting its JoinHandle via the pub field, then polling.
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        vec![MockOutcome::SleepForever],
        Arc::clone(&recorded),
        0,
    ));
    let _ = d.spawn_turn("p1".into(), runner);
    // Abort without taking — handle stays in current_query so poll will see
    // it as finished with a cancelled JoinError.
    if let Some(h) = d.current_query.as_ref() {
        h.abort();
    }
    // Give tokio a tick to mark the handle finished.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if d
            .current_query
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    let outcome = d.poll_completion().expect("handle should be finished");
    assert!(
        matches!(outcome, TurnOutcome::Cancelled),
        "expected Cancelled on aborted handle"
    );
    assert!(!d.is_busy());
}

#[tokio::test]
async fn test_poll_completion_panic_branch() {
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        vec![MockOutcome::Panic("boom!".into())],
        Arc::clone(&recorded),
        1,
    ));
    let _ = d.spawn_turn("p1".into(), runner);
    let outcomes = drain_until_idle(&mut d).await;
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        TurnOutcome::Failed(msg) => {
            assert!(
                msg.starts_with("panic:"),
                "expected message to start with 'panic:', got: {msg}"
            );
            assert!(
                msg.contains("boom!"),
                "expected panic message to preserve 'boom!', got: {msg}"
            );
        }
        _ => panic!("expected Failed(panic: ...) for panic branch"),
    }
    assert!(!d.is_busy());
}

#[tokio::test]
async fn test_poll_completion_drains_queue_fifo() {
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    // 6 successful outcomes (A + P1..P5).
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        vec![
            MockOutcome::Success,
            MockOutcome::Success,
            MockOutcome::Success,
            MockOutcome::Success,
            MockOutcome::Success,
            MockOutcome::Success,
        ],
        Arc::clone(&recorded),
        20, // each turn takes ~20ms
    ));
    // First dispatch spawns the task; subsequent ones queue.
    let _ = d.spawn_turn("A".into(), Arc::clone(&runner));
    let _ = d.spawn_turn("P1".into(), Arc::clone(&runner));
    let _ = d.spawn_turn("P2".into(), Arc::clone(&runner));
    let _ = d.spawn_turn("P3".into(), Arc::clone(&runner));
    let _ = d.spawn_turn("P4".into(), Arc::clone(&runner));
    let _ = d.spawn_turn("P5".into(), Arc::clone(&runner));
    assert_eq!(d.queue_len(), 5);
    let outcomes = drain_until_idle(&mut d).await;
    assert_eq!(outcomes.len(), 6);
    for o in &outcomes {
        assert!(matches!(o, TurnOutcome::Completed));
    }
    assert!(!d.is_busy());
    assert_eq!(d.queue_len(), 0);
    let got = recorded.lock().unwrap().clone();
    assert_eq!(
        got,
        vec![
            "A".to_string(),
            "P1".to_string(),
            "P2".to_string(),
            "P3".to_string(),
            "P4".to_string(),
            "P5".to_string(),
        ],
        "FIFO order violated"
    );
}

#[tokio::test]
async fn test_poll_completion_preserves_no_loss() {
    let mut d = make_dispatcher();
    let recorded = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    // 11 successes: 1 original + 10 burst.
    let runner: Arc<dyn TurnRunner> = Arc::new(ConfigurableRunner::new(
        (0..11).map(|_| MockOutcome::Success).collect(),
        Arc::clone(&recorded),
        5,
    ));
    let _ = d.spawn_turn("A".into(), Arc::clone(&runner));
    for i in 0..10 {
        let _ = d.spawn_turn(format!("burst-{i}"), Arc::clone(&runner));
    }
    assert_eq!(d.queue_len(), 10);
    let outcomes = drain_until_idle(&mut d).await;
    assert_eq!(outcomes.len(), 11, "expected 11 outcomes (no loss)");
    for o in &outcomes {
        assert!(matches!(o, TurnOutcome::Completed));
    }
    let got = recorded.lock().unwrap();
    assert_eq!(got.len(), 11, "expected 11 recorded prompts");
    assert_eq!(got[0], "A");
    for i in 0..10 {
        assert_eq!(got[i + 1], format!("burst-{i}"));
    }
}
