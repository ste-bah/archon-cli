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
    cancel_slot: Arc<Mutex<Option<CancellationToken>>>,
    /// v0.1.23: AutoCapture instance for per-turn regex-based memory detection.
    auto_capture: Option<Arc<AutoCapture>>,
    /// GNN auto-trainer — when present, the auto-capture site below records each
    /// stored memory so the background loop's triggers fire correctly.
    auto_trainer: Option<Arc<AutoTrainer>>,
}

impl AgentHandle {
    pub fn new(
        agent: Arc<Mutex<Agent>>,
        auto_capture: Option<Arc<AutoCapture>>,
        auto_trainer: Option<Arc<AutoTrainer>>,
    ) -> Self {
        Self {
            agent,
            cancel_slot: Arc::new(Mutex::new(None)),
            auto_capture,
            auto_trainer,
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
            // v0.1.23: AutoCapture — regex-based memory detection at turn boundary.
            if let Some(ref capture) = self.auto_capture {
                let guard = agent.lock().await;
                let turn_num = guard.turn_number() as usize;
                let captured = capture.detect(&prompt, turn_num);
                if !captured.is_empty() {
                    let mut recent: Vec<archon_pipeline::capture::CapturedMemory> = Vec::new();
                    for mem in captured {
                        if !AutoCapture::is_duplicate(&mem, &recent) {
                            if let Some(ref memory) = guard.memory_handle() {
                                let stored = memory.store_memory(
                                    &mem.content,
                                    &mem.content.chars().take(80).collect::<String>(),
                                    archon_memory::types::MemoryType::Fact,
                                    mem.confidence as f64,
                                    &["auto-captured".to_string()],
                                    "auto_capture",
                                    "",
                                );
                                // Reference: auto_trainer.rs::record_memory.
                                // Only count successful stores so triggers reflect
                                // real memory accumulation.
                                if stored.is_ok() {
                                    if let Some(ref at) = self.auto_trainer {
                                        at.record_memory();
                                    }
                                }
                            }
                            recent.push(mem);
                        }
                    }
                }
                drop(guard);
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
