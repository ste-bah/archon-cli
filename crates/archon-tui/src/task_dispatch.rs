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

    /// Spawn (or queue) a turn to run the given prompt.
    /// Spec: 02-technical-spec.md:101-110.
    ///
    /// Contract:
    /// 1. If `self.current_query.is_some()` → push a [`QueuedPrompt`] onto
    ///    `pending_queue` and return [`DispatchResult::Queued`].
    /// 2. Otherwise, clone the runner `Arc` and hand the clone to
    ///    `tokio::spawn(async move { runner.run_turn(prompt).await })`. Store
    ///    the `JoinHandle` in `self.current_query` and return
    ///    [`DispatchResult::Running`] with `spawned_at = Instant::now()`.
    ///
    /// This function MUST NOT `.await` the spawned handle — that would
    /// re-introduce the exact input-loop blockage this subsystem exists to
    /// remove (see REQ-TUI-LOOP-001 / AC-EVENTLOOP-02).
    pub fn spawn_turn(
        &mut self,
        prompt: String,
        runner: Arc<dyn TurnRunner>,
    ) -> DispatchResult {
        if self.current_query.is_some() {
            let qp = QueuedPrompt {
                prompt,
                agent_id: None,
                submitted_at: Instant::now(),
            };
            self.pending_queue.push_back(qp);
            return DispatchResult::Queued;
        }
        let spawned_at = Instant::now();
        // Clone the Arc so the spawned future owns a 'static handle to the
        // runner. The `&self` borrow in `TurnRunner::run_turn` is tied to
        // the Arc's lifetime; tokio::spawn requires 'static, hence the clone.
        let runner_clone = Arc::clone(&runner);
        let handle = tokio::spawn(async move {
            runner_clone.run_turn(prompt).await
        });
        self.current_query = Some(handle);
        DispatchResult::Running { spawned_at }
    }

    /// Abort the in-flight turn, if any.
    /// Spec: 02-technical-spec.md:112-119.
    ///
    /// Contract:
    /// 1. If `current_query` is `None`, return [`CancelOutcome::NoInflight`].
    /// 2. Otherwise, take ownership of the [`tokio::task::JoinHandle`], call
    ///    `abort()` on it, and return [`CancelOutcome::Aborted`] carrying the
    ///    wall-clock latency of the abort path (NFR-TUI-PERF-003).
    ///
    /// This function MUST NOT `.await` the handle after `abort()`.
    /// `JoinHandle::abort` is cooperative: the spawned task is cancelled at
    /// its next `.await` point. The render loop (TASK-TUI-103) detects
    /// completion via `JoinError::is_cancelled()` on its next poll. Awaiting
    /// here would re-introduce the input-loop blockage this subsystem exists
    /// to remove (REQ-TUI-LOOP-001 / AC-EVENTLOOP-02).
    pub fn cancel_current(&mut self) -> CancelOutcome {
        let start = std::time::Instant::now();
        match self.current_query.take() {
            None => CancelOutcome::NoInflight,
            Some(h) => {
                h.abort();
                let elapsed_ms = start.elapsed().as_millis() as u64;
                CancelOutcome::Aborted { elapsed_ms }
            }
        }
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
#[path = "task_dispatch_tests.rs"]
mod tests;
