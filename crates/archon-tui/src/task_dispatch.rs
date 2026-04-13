//! TASK-TUI-100: Scaffold for `AgentDispatcher` and associated data models.
//!
//! This module owns the lifecycle of an in-flight agent turn so the input
//! handler never blocks. It is **scaffold-only** in TASK-TUI-100: every
//! method body is `todo!()` except trivial accessors (`queue_len`, `is_busy`).
//! Behaviour lands in TASK-TUI-101 (`spawn_turn`), TASK-TUI-102
//! (`cancel_current`), TASK-TUI-103 (`poll_completion`), and TASK-TUI-104
//! (`switch_agent`).
//!
//! Spec: `project-tasks/archon-fixes/tui_fixes/02-technical-spec.md` lines
//! 65-97 (data_models) and 101-127 (api_contracts). Line budget: <300.
//!
//! ## Spec deviation (approved 2026-04-13)
//!
//! The spec says `Arc<dyn Agent>` where `Agent` is a trait. In this codebase,
//! `archon_core::agent::Agent` is a concrete struct (see
//! `crates/archon-core/src/agent.rs:318`), not a trait. To preserve the spec
//! intent (decoupled dispatch, fakeable in tests) without inventing a trait
//! in the wrong crate, we define a local trait [`TurnRunner`] here. The real
//! implementation wrapping `archon_core::agent::Agent` ships in TASK-TUI-107.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use archon_core::agent::AgentEvent;
use tokio::sync::mpsc::UnboundedSender;

/// Abstraction over "something that can run a single agent turn". Defined
/// locally (see module-level spec deviation note) so the dispatcher is
/// decoupled from `archon_core::agent::Agent` and fakeable in tests.
pub trait TurnRunner: Send + Sync {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

/// Placeholder router trait used by `AgentDispatcher::switch_agent`. The
/// concrete implementation is owned elsewhere (SPEC-TUI-SUBAGENT); here we
/// only need an object-safe trait so the struct field type resolves.
pub trait AgentRouter: Send + Sync {
    fn switch(&self, agent_id: &str) -> anyhow::Result<()>;
}

/// A user prompt that arrived while an agent turn was already running and
/// is waiting FIFO in `AgentDispatcher::pending_queue`.
pub struct QueuedPrompt {
    pub prompt: String,
    pub agent_id: Option<String>,
    pub submitted_at: Instant,
}

/// Result of `AgentDispatcher::cancel_current`. Carries the wall-clock
/// latency of the abort for NFR-TUI-PERF-003 (<500ms target).
pub enum CancelOutcome {
    /// `/cancel` was invoked with nothing running.
    NoInflight,
    /// The tracked `JoinHandle` was aborted.
    Aborted { elapsed_ms: u64 },
}

/// Result of `AgentDispatcher::spawn_turn`.
pub enum DispatchResult {
    /// An agent turn was already running; the prompt was queued FIFO.
    Queued,
    /// A new tokio task was spawned to run the turn.
    Running { spawned_at: Instant },
    /// The dispatcher refused the prompt (e.g. invalid state).
    Rejected(String),
}

/// Result of polling a completed turn via `AgentDispatcher::poll_completion`.
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed(String),
}

/// Owns the lifecycle of an in-flight agent turn. The TUI event loop drives
/// this dispatcher so the input handler never blocks on `process_message`.
///
/// Spec: 02-technical-spec.md:69-78.
pub struct AgentDispatcher {
    /// Handle to the in-flight agent turn, if any. Aborted by `/cancel`.
    pub current_query: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    /// FIFO queue of prompts submitted while an agent turn was in flight.
    pub pending_queue: std::collections::VecDeque<QueuedPrompt>,
    /// Router handle used for agent switching (TASK-TUI-104).
    pub router: std::sync::Arc<dyn AgentRouter>,
    /// Unbounded producer side of the agent event channel
    /// (see TECH-TUI-EVENTCHANNEL).
    pub agent_event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
}

impl AgentDispatcher {
    /// Construct a new dispatcher with no in-flight turn and an empty queue.
    pub fn new(
        router: Arc<dyn AgentRouter>,
        agent_event_tx: UnboundedSender<AgentEvent>,
    ) -> Self {
        Self {
            current_query: None,
            pending_queue: VecDeque::new(),
            router,
            agent_event_tx,
        }
    }

    /// Spawn (or queue) a turn to run the given prompt. Body lands in
    /// TASK-TUI-101. Spec: 02-technical-spec.md:101-110.
    pub fn spawn_turn(
        &mut self,
        _prompt: String,
        _runner: Arc<dyn TurnRunner>,
    ) -> DispatchResult {
        // Shut the unused-field lint up while still making sure the field
        // types resolve. Body lands in TASK-TUI-101.
        let _ = &self.current_query;
        let _ = &self.pending_queue;
        let _ = &self.agent_event_tx;
        todo!("TASK-TUI-101")
    }

    /// Abort the in-flight turn, if any. Body lands in TASK-TUI-102.
    /// Spec: 02-technical-spec.md:112-119.
    pub fn cancel_current(&mut self) -> CancelOutcome {
        let _ = &self.current_query;
        todo!("TASK-TUI-102")
    }

    /// Poll the in-flight `JoinHandle` for completion. Body lands in
    /// TASK-TUI-103. Spec: 02-technical-spec.md:121-127.
    pub fn poll_completion(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> Option<TurnOutcome> {
        let _ = &self.current_query;
        let _ = &self.pending_queue;
        todo!("TASK-TUI-103")
    }

    /// Switch the active agent via the router. Body lands in TASK-TUI-104.
    pub fn switch_agent(&mut self, _agent_id: &str) -> anyhow::Result<()> {
        let _ = &self.router;
        todo!("TASK-TUI-104")
    }

    /// Number of prompts currently waiting in the FIFO queue.
    pub fn queue_len(&self) -> usize {
        self.pending_queue.len()
    }

    /// `true` iff an agent turn is in flight.
    pub fn is_busy(&self) -> bool {
        self.current_query.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let r2 = DispatchResult::Running {
            spawned_at: std::time::Instant::now(),
        };
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
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>,
        > {
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
}
