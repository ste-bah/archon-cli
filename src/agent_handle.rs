//! TASK-TUI-107: AgentHandle adapter bridging `Arc<Mutex<archon_core::agent::Agent>>`
//! to `archon_tui::TurnRunner`.
//!
//! ## Spec Deviation (TASK-TUI-107, approved 2026-04-14)
//!
//! 1. Line numbers wrong. Spec said 3292/3742-3745. Reality: input loop at
//!    3759, process_message sites at 4258 and 4335. Located via grep.
//!
//! 2. Spec premise partial. Spec framed TUI-107 as "swap blocking .await in
//!    input loop." Recon found both .await sites already inside tokio::spawn
//!    blocks. Real blocking pattern is `handle.await` on prior turn's
//!    JoinHandle at 4243-4245 (serialization via wait-on-prior, not
//!    inline-await). Fix is architectural: delete `current_agent_task_inner`
//!    + handle-tracking + wait-on-prior pattern wholesale, replace with
//!    `AgentDispatcher` ownership.
//!
//! 3. Trait mismatch: spec says `Arc<dyn Agent>`. TUI-100 deviation applies:
//!    `Arc<dyn TurnRunner>`. This `AgentHandle` wraps
//!    `Arc<Mutex<archon_core::agent::Agent>>`. Adapter locks + awaits
//!    `process_message` inside `run_turn`, maps `AgentLoopError` to anyhow.
//!
//! 4. No `run_event_loop` call. Spec mentions it as option; `main.rs` still
//!    owns slash-command routing, session restore, skill dispatch not in
//!    `run_event_loop`'s scope. Full integration deferred to
//!    SPEC-TUI-MODULARIZATION. TUI-107 uses `AgentDispatcher` directly +
//!    minimal `tokio::select!` conversion (input arm + 16ms tick arm).
//!
//! 5. `agent_event_tx` scope: exists at 3162, not currently captured into
//!    input loop closure. Coder plumbs it through (small additive change,
//!    not a phase-2 prereq land).
//!
//! 6. `NoopAgentRouter` placeholder: no real multi-agent router exists yet.
//!    `/agent` switching is not implemented by TUI-107 scope.

use std::pin::Pin;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::{AgentRouter, TurnRunner};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Adapter bridging main.rs's `Arc<Mutex<Agent>>` to archon_tui's
/// `TurnRunner` trait.
///
/// Why this adapter exists:
/// - `archon_core::agent::Agent` is a concrete struct, not a trait
///   (TUI-100 deviation).
/// - `archon_tui` cannot depend on `archon_core` directly — that would pin
///   `archon_tui` to a single agent impl and break its clean dependency
///   direction.
/// - Therefore the binary crate (`archon-cli-workspace`) owns the trait
///   impl, wrapping the concrete `Agent` in an `Arc<Mutex<_>>` and
///   implementing `run_turn` as lock + `set_cancel_token` +
///   `process_message` + anyhow coercion.
///
/// ## Cancel-token side channel
///
/// The adapter owns a shared `Arc<Mutex<Option<CancellationToken>>>` slot.
/// Each `run_turn` creates a fresh `CancellationToken`, stores it in the
/// slot, calls `agent.set_cancel_token(Some(..))` so `ToolContext.cancel_parent`
/// propagates into subagent `child_token()` chains, runs `process_message`,
/// then clears both the slot and the agent's token.
///
/// On Ctrl+C (`__cancel__`), the main input loop calls `self.fire_cancel()`
/// which fires the stored token (reaching spawned children) in addition to
/// calling `AgentDispatcher::cancel_current()` which aborts the outer
/// JoinHandle (reaching the turn future at its next `.await`).
pub struct AgentHandle {
    agent: Arc<Mutex<Agent>>,
    cancel_slot: Arc<Mutex<Option<CancellationToken>>>,
}

impl AgentHandle {
    pub fn new(agent: Arc<Mutex<Agent>>) -> Self {
        Self {
            agent,
            cancel_slot: Arc::new(Mutex::new(None)),
        }
    }

    /// Fire the CancellationToken associated with the in-flight turn, if
    /// any. Synchronous / non-blocking; uses `try_lock` so Ctrl+C never
    /// waits on a contended lock.
    pub fn fire_cancel(&self) {
        if let Ok(guard) = self.cancel_slot.try_lock() {
            if let Some(ref token) = *guard {
                token.cancel();
                tracing::info!("AgentHandle: fired CancellationToken on current turn");
            }
        }
    }
}

impl TurnRunner for AgentHandle {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let agent = self.agent.clone();
        let cancel_slot = self.cancel_slot.clone();
        Box::pin(async move {
            // TASK-AGS-107: set a fresh CancellationToken so
            // ToolContext.cancel_parent propagates into subagent
            // child_token() chains for the duration of this turn.
            let cancel = CancellationToken::new();
            {
                let mut slot = cancel_slot.lock().await;
                *slot = Some(cancel.clone());
            }
            let mut guard = agent.lock().await;
            guard.set_cancel_token(Some(cancel));
            let result = guard
                .process_message(&prompt)
                .await
                .map_err(anyhow::Error::from);
            guard.set_cancel_token(None);
            drop(guard);
            {
                let mut slot = cancel_slot.lock().await;
                *slot = None;
            }
            result
        })
    }
}

/// Placeholder router until multi-agent switching lands in phase-2/3.
/// TUI-107 scope does not implement `/agent` switching.
pub struct NoopAgentRouter;

impl AgentRouter for NoopAgentRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
