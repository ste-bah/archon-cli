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
use archon_pipeline::capture::AutoCapture;
// Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs — record_memory() bumps the
// GNN auto-trainer's memory counter so triggers fire when threshold is met.
use archon_pipeline::learning::gnn::auto_trainer::AutoTrainer;
use archon_tui::{AgentRouter, TurnRunner};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Adapter bridging main.rs's `Arc<Mutex<Agent>>` to archon_tui's
/// `TurnRunner` trait.
pub struct AgentHandle {
    agent: Arc<Mutex<Agent>>,
    session_id: String,
    /// Bug-fix 2026-05-12: switched from `tokio::sync::Mutex` to
    /// `std::sync::Mutex` so `fire_cancel` can lock synchronously without
    /// the previous `try_lock` silent-no-op on contention. Both critical
    /// sections in `run_turn` (slot set / slot clear) hold the lock for
    /// only a single synchronous mutation — never across an `.await` — so
    /// using std::sync::Mutex from async code is sound here.
    cancel_slot: Arc<std::sync::Mutex<Option<CancellationToken>>>,
    /// v0.1.23: AutoCapture instance for per-turn regex-based memory detection.
    auto_capture: Option<Arc<AutoCapture>>,
    /// GNN auto-trainer — when present, the auto-capture site below records each
    /// stored memory so the background loop's triggers fire correctly.
    auto_trainer: Option<Arc<AutoTrainer>>,
}

impl AgentHandle {
    pub fn new(
        agent: Arc<Mutex<Agent>>,
        session_id: String,
        auto_capture: Option<Arc<AutoCapture>>,
        auto_trainer: Option<Arc<AutoTrainer>>,
    ) -> Self {
        Self {
            agent,
            session_id,
            cancel_slot: Arc::new(std::sync::Mutex::new(None)),
            auto_capture,
            auto_trainer,
        }
    }

    pub fn scoped_turn_runner(
        self: &Arc<Self>,
        guardrail_action_id: String,
    ) -> Arc<dyn TurnRunner> {
        Arc::new(GuardrailTurnRunner {
            inner: Arc::clone(self),
            guardrail_action_id,
        })
    }

    fn run_turn_scoped<'a>(
        &'a self,
        prompt: String,
        guardrail_action_id: Option<String>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let agent = self.agent.clone();
        let session_id = self.session_id.clone();
        let cancel_slot = self.cancel_slot.clone();
        Box::pin(async move {
            if let Some(action_id) = guardrail_action_id.as_deref() {
                crate::command::world_model::activate_guardrail_for_action(&session_id, action_id);
            }
            let cancel = CancellationToken::new();
            {
                let mut slot = cancel_slot.lock().unwrap_or_else(|p| p.into_inner());
                *slot = Some(cancel.clone());
            }
            if let Some(ref capture) = self.auto_capture {
                let guard = agent.lock().await;
                let turn_num = guard.turn_number() as usize;
                let captured = capture.detect(&prompt, turn_num);
                if !captured.is_empty() {
                    let mut recent: Vec<archon_pipeline::capture::CapturedMemory> = Vec::new();
                    for mem in captured {
                        if !AutoCapture::is_duplicate(&mem, &recent) {
                            if let Some(memory) = guard.memory_handle() {
                                let stored = memory.store_memory(
                                    &mem.content,
                                    &mem.content.chars().take(80).collect::<String>(),
                                    archon_memory::types::MemoryType::Fact,
                                    mem.confidence as f64,
                                    &["auto-captured".to_string()],
                                    "auto_capture",
                                    "",
                                );
                                if stored.is_ok()
                                    && let Some(ref at) = self.auto_trainer
                                {
                                    at.record_memory();
                                }
                            }
                            recent.push(mem);
                        }
                    }
                }
                drop(guard);
            }

            let mut guard = agent.lock().await;
            guard.set_guardrail_action_id(guardrail_action_id);
            guard.set_cancel_token(Some(cancel));
            let result = guard
                .process_message(&prompt)
                .await
                .map_err(anyhow::Error::from);
            guard.set_cancel_token(None);
            guard.set_guardrail_action_id(None);
            drop(guard);
            {
                let mut slot = cancel_slot.lock().unwrap_or_else(|p| p.into_inner());
                *slot = None;
            }
            result
        })
    }

    /// Fire the CancellationToken associated with the in-flight turn, if
    /// any. Synchronous; takes the std mutex briefly. Logs the outcome so
    /// silent-no-op cancellation failures are visible in traces.
    pub fn fire_cancel(&self) {
        let guard = match self.cancel_slot.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("AgentHandle: cancel_slot mutex poisoned; recovering inner state");
                poisoned.into_inner()
            }
        };
        match guard.as_ref() {
            Some(token) => {
                token.cancel();
                tracing::info!("AgentHandle: fired CancellationToken on current turn");
            }
            None => {
                tracing::debug!("AgentHandle: fire_cancel called but no in-flight turn");
            }
        }
    }
}

impl TurnRunner for AgentHandle {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        self.run_turn_scoped(prompt, None)
    }
}

struct GuardrailTurnRunner {
    inner: Arc<AgentHandle>,
    guardrail_action_id: String,
}

impl TurnRunner for GuardrailTurnRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        self.inner
            .run_turn_scoped(prompt, Some(self.guardrail_action_id.clone()))
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
