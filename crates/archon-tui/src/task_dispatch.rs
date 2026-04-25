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

use archon_core::agent::{AgentEvent, TimestampedEvent};
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
///
/// ## Spec deviation (TASK-TUI-103, approved 2026-04-14)
///
/// The spec (TASK-TUI-103) says to add `agent: Arc<dyn Agent>` so drained
/// prompts can be re-dispatched against the originally-targeted agent. But
/// `archon_core::agent::Agent` is a concrete struct, not a trait — the same
/// reason TASK-TUI-100 introduced the local [`TurnRunner`] trait. We carry
/// that deviation forward: the field is `runner: Arc<dyn TurnRunner>` rather
/// than `agent: Arc<dyn Agent>`. The real `AgentHandle: TurnRunner` wrapper
/// still lands in TASK-TUI-107.
pub struct QueuedPrompt {
    pub prompt: String,
    pub agent_id: Option<String>,
    pub submitted_at: Instant,
    pub runner: Arc<dyn TurnRunner>,
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
    pub agent_event_tx: tokio::sync::mpsc::UnboundedSender<TimestampedEvent>,
}

impl AgentDispatcher {
    /// Construct a new dispatcher with no in-flight turn and an empty queue.
    pub fn new(
        router: Arc<dyn AgentRouter>,
        agent_event_tx: UnboundedSender<TimestampedEvent>,
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
    pub fn spawn_turn(&mut self, prompt: String, runner: Arc<dyn TurnRunner>) -> DispatchResult {
        if self.current_query.is_some() {
            let qp = QueuedPrompt {
                prompt,
                agent_id: None,
                submitted_at: Instant::now(),
                runner,
            };
            self.pending_queue.push_back(qp);
            return DispatchResult::Queued;
        }
        self.spawn_turn_internal(prompt, runner)
    }

    /// Internal helper that unconditionally spawns a tokio task for a turn.
    /// This is the single spawn code path shared by [`spawn_turn`] (direct
    /// dispatch) and [`poll_completion`] (queue drain) — so queued prompts
    /// are indistinguishable from direct prompts (TASK-TUI-103 drain
    /// contract; sherlock-probe #4).
    fn spawn_turn_internal(
        &mut self,
        prompt: String,
        runner: Arc<dyn TurnRunner>,
    ) -> DispatchResult {
        let spawned_at = Instant::now();
        // Clone the Arc so the spawned future owns a 'static handle to the
        // runner. The `&self` borrow in `TurnRunner::run_turn` is tied to
        // the Arc's lifetime; tokio::spawn requires 'static, hence the clone.
        let runner_clone = Arc::clone(&runner);
        let handle = tokio::spawn(async move { runner_clone.run_turn(prompt).await });
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

    /// Poll the in-flight `JoinHandle` for completion and, on completion,
    /// drain one prompt from the FIFO queue. Spec: 02-technical-spec.md:121-127.
    ///
    /// ## Spec deviation (TASK-TUI-103, approved 2026-04-14)
    ///
    /// The TASK-TUI-100 scaffold declared
    /// `poll_completion(&mut self, _cx: &mut std::task::Context<'_>)`, but
    /// the Context was never consumed and the in-scope text of TASK-TUI-103
    /// line 39 declares `poll_completion(&mut self) -> Option<TurnOutcome>`
    /// — a sync, non-blocking wrapper driven by
    /// [`tokio::task::JoinHandle::is_finished`]. We drop the `_cx` argument
    /// here. Nothing calls this yet (TASK-TUI-106 wires the event loop), so
    /// the surface change is free.
    ///
    /// ## Ordering guarantee (sherlock-probe #2)
    ///
    /// This method enforces strict slot-clear → outcome-extract → next-spawn
    /// ordering:
    /// 1. `self.current_query.take()` clears the slot synchronously. Any
    ///    observer seeing `is_busy() == false` past this line is consistent.
    /// 2. [`futures_util::FutureExt::now_or_never`] retrieves the finished
    ///    handle's `Result` without awaiting (we already verified
    ///    `handle.is_finished()` so readiness is guaranteed).
    /// 3. The queue head is popped and re-dispatched via
    ///    [`Self::spawn_turn_internal`] so direct dispatch and drain go
    ///    through the same single spawn code path.
    ///
    /// ## Non-blocking contract (sherlock-probe #3)
    ///
    /// The body contains zero `.await` calls. This MUST remain true — it is
    /// the entire reason [`poll_completion`] is callable from inside a
    /// `tokio::select!` arm in the event loop (TASK-TUI-106) without
    /// starving other branches. If you add `.await`, you have broken the
    /// subsystem's reason for existing.
    ///
    /// ## Panic mapping (sherlock-probe #6)
    ///
    /// [`TurnOutcome`] from TASK-TUI-100 has no `Panicked` variant. When the
    /// inner task panics, [`tokio::task::JoinError::is_panic`] is true and
    /// [`JoinError::into_panic`] returns a `Box<dyn Any + Send>`. We
    /// downcast to `String` then `&'static str` then fall back to a static
    /// placeholder, format as `"panic: <msg>"`, and emit
    /// [`TurnOutcome::Failed`]. We do NOT call `std::panic::resume_unwind`
    /// — that would crash the event loop, defeating the whole point of the
    /// dispatcher. We also do NOT silently swallow the payload — the panic
    /// string MUST reach the caller.
    pub fn poll_completion(&mut self) -> Option<TurnOutcome> {
        // Step 1 (slot-clear): if nothing running, or running but unfinished,
        // exit early WITHOUT touching any state. Otherwise take() the handle
        // so the slot is None before we emit the outcome.
        let handle = match self.current_query.as_ref() {
            None => return None,
            Some(h) if !h.is_finished() => return None,
            Some(_) => self.current_query.take().expect("just checked Some above"),
        };

        // Step 2 (outcome-extract): the handle is finished, so polling it
        // once must succeed synchronously. `now_or_never` returns
        // `Some(value)` when the future is immediately ready and `None` when
        // it would have to wait — impossible for a finished tokio handle in
        // practice, but handled defensively below.
        use futures_util::future::FutureExt;
        let outcome = match handle.now_or_never() {
            Some(Ok(Ok(()))) => TurnOutcome::Completed,
            Some(Ok(Err(app_err))) => TurnOutcome::Failed(format!("{app_err}")),
            Some(Err(join_err)) if join_err.is_cancelled() => TurnOutcome::Cancelled,
            Some(Err(join_err)) if join_err.is_panic() => {
                let payload = join_err.into_panic();
                let msg = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        payload
                            .downcast_ref::<&'static str>()
                            .map(|s| (*s).to_string())
                    })
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                TurnOutcome::Failed(format!("panic: {msg}"))
            }
            Some(Err(_)) => {
                // Defensive: tokio currently guarantees every JoinError is
                // either cancelled or a panic. If that invariant ever
                // changes we still keep the event loop alive by mapping to
                // Failed rather than panicking inside poll_completion.
                TurnOutcome::Failed("unknown JoinError variant".to_string())
            }
            None => {
                // `is_finished()` returned true but `now_or_never` said the
                // future was Pending. Should be impossible for a finished
                // tokio handle; we already consumed the handle via take()
                // so we cannot put it back. Surface as Failed to keep the
                // state machine advancing rather than wedging the loop.
                TurnOutcome::Failed(
                    "poll_completion: is_finished=true but now_or_never returned None".to_string(),
                )
            }
        };

        // Step 3 (next-spawn): drain one FIFO entry via the SAME internal
        // spawn path used by direct dispatch. We intentionally ignore the
        // returned DispatchResult — the caller observes the new state via
        // the next poll_completion() / is_busy() call.
        if let Some(next_qp) = self.pending_queue.pop_front() {
            let _ = self.spawn_turn_internal(next_qp.prompt, next_qp.runner);
        }

        Some(outcome)
    }

    /// Switch the active agent via the router.
    /// Spec: 02-technical-spec.md:51
    /// (`TuiEvent::SlashAgent → router.switch(agent_id) (non-blocking)`).
    ///
    /// ## Thin delegation model
    ///
    /// This is **Option (c)** from the TASK-TUI-104 design space: a pure
    /// delegation to `self.router.switch(agent_id)` that returns the router's
    /// result verbatim. The dispatcher holds no implicit "current runner"
    /// state — `spawn_turn` takes an explicit `Arc<dyn TurnRunner>` from its
    /// caller, so the router's updated selection only affects *future*
    /// caller-driven dispatches that read from it. Nothing queued and nothing
    /// in flight is touched.
    ///
    /// ## Zero-mutation contract (sherlock-probe #2, corrected anchor)
    ///
    /// This function MUST NOT touch:
    /// - `self.current_query` — the in-flight `JoinHandle` keeps streaming
    ///   into `agent_event_tx` until it completes or is cancelled, per
    ///   REQ-TUI-LOOP-005 and AC-EVENTLOOP-06. Aborting here would strand
    ///   the agent mid-turn; awaiting here would re-introduce the input-loop
    ///   blockage the dispatcher exists to remove.
    /// - `self.pending_queue` — queued prompts already carry their
    ///   `runner: Arc<dyn TurnRunner>` captured at enqueue time (TASK-TUI-103
    ///   deviation). A switch does NOT rewrite those captured runners; the
    ///   capture-at-enqueue contract is preserved transitively across the
    ///   switch boundary.
    /// - `self.agent_event_tx` — the event channel outlives individual
    ///   turns and agent switches.
    ///
    /// ## Non-blocking contract
    ///
    /// Zero `.await` calls. `AgentRouter::switch` is a synchronous trait
    /// method by design so this whole path stays sync-callable from the
    /// event loop (TASK-TUI-106).
    pub fn switch_agent(&mut self, agent_id: &str) -> anyhow::Result<()> {
        self.router.switch(agent_id)
    }

    /// Read-only accessor: `true` iff a current turn is tracked and its
    /// [`tokio::task::JoinHandle`] has not yet finished.
    ///
    /// Used by TASK-TUI-106's event-loop dispatch and by TASK-TUI-104 tests
    /// that need to assert an in-flight turn survived a `switch_agent` call
    /// without the dispatcher taking the handle.
    pub fn current_handle_is_inflight(&self) -> bool {
        self.current_query
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
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
