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
    let q = QueuedPrompt {
        prompt: "hello".into(),
        agent_id: None,
        submitted_at: std::time::Instant::now(),
    };
    assert_eq!(q.prompt, "hello");
    assert!(q.agent_id.is_none());
    let _ = q.submitted_at;
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
