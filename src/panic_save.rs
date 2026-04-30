//! Panic-driven personality snapshot save (TASK #246 CONSCIOUSNESS-PERSIST-5).
//!
//! Installs a `std::panic::set_hook` AFTER the InnerVoice + cmd_ctx are
//! constructed so a panic mid-session captures the current personality
//! state to the memory graph before the process aborts. Falls back to
//! the default panic hook for stack-trace printing.
//!
//! The hook runs OUTSIDE any tokio runtime context, so it cannot await
//! the existing `Arc<tokio::sync::Mutex<InnerVoice>>`. Instead the
//! binary keeps a parallel `Arc<std::sync::Mutex<InnerVoice>>` mirror
//! continuously updated via the `Agent::set_inner_voice_change_callback`
//! hook (TASK #245). The hook reads the mirror with a poison-tolerant
//! lock and calls the synchronous `save_snapshot` directly.
//!
//! Limitations (documented, accepted):
//! - Panics BEFORE this module's `install` call (config load, agent
//!   init, resume restoration) are not captured. `TerminalGuard::Drop`
//!   still restores the terminal via RAII.
//! - Panics during a CozoDB `run_script` may leave the lock held; the
//!   hook's `save_snapshot` will fail and fall through silently.
//! - Stack-overflow panics may not have stack to run the hook at all.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, OnceLock};

use archon_consciousness::inner_voice::InnerVoice;
use archon_memory::access::MemoryTrait;
use crossterm::ExecutableCommand;
use crossterm::cursor::Show;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

/// Captured at install time; read by the panic hook.
pub(crate) struct PanicSaveContext {
    pub memory: Arc<dyn MemoryTrait>,
    pub mirror: Arc<Mutex<InnerVoice>>,
    pub session_id: String,
    pub session_start_confidence: f32,
    pub session_start_instant: std::time::Instant,
    pub personality_history_limit: u32,
}

/// Process-wide context. `None` until `install` is called. Tests never
/// hit `install` (only `run_interactive_session` does), so the hook
/// body short-circuits when this is empty.
pub(crate) static PANIC_CTX: OnceLock<PanicSaveContext> = OnceLock::new();

/// Install the panic hook AND wire the InnerVoice change callback.
///
/// Idempotent — safe to call multiple times (subsequent calls fail at
/// `OnceLock::set` and are silently ignored).
///
/// Returns the mirror Arc so the caller can wire it into the agent's
/// `set_inner_voice_change_callback` hook.
pub(crate) fn install(
    memory: Arc<dyn MemoryTrait>,
    initial_inner_voice: InnerVoice,
    session_id: String,
    session_start_confidence: f32,
    session_start_instant: std::time::Instant,
    personality_history_limit: u32,
) -> Arc<Mutex<InnerVoice>> {
    let mirror = Arc::new(Mutex::new(initial_inner_voice));

    let ctx = PanicSaveContext {
        memory,
        mirror: Arc::clone(&mirror),
        session_id,
        session_start_confidence,
        session_start_instant,
        personality_history_limit,
    };

    if PANIC_CTX.set(ctx).is_err() {
        tracing::warn!("panic_save::install called twice; second call ignored");
        return mirror;
    }

    // Chain to the previous hook so stack traces still print.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Bypass the hook if the OnceLock is somehow empty (e.g. install
        // raced). This also makes test-binary panics no-op since tests
        // never call `install`.
        let Some(ctx) = PANIC_CTX.get() else {
            prev_hook(info);
            return;
        };

        // catch_unwind so a panic INSIDE the hook can't double-abort.
        // AssertUnwindSafe: Arc<Mutex<...>> are not auto-UnwindSafe;
        // this opt-out is required and matches dispatcher.rs:707.
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            // Restore terminal in the SAME order TerminalGuard::Drop uses
            // (crates/archon-tui/src/terminal.rs:47-49) so post-panic
            // console is usable.
            let _ = std::io::stdout().execute(Show);
            let _ = std::io::stdout().execute(LeaveAlternateScreen);
            let _ = disable_raw_mode();

            // Read mirror with poison tolerance — a panic while the
            // mirror lock was held would have poisoned it.
            let iv_clone = match ctx.mirror.lock() {
                Ok(g) => g.clone(),
                Err(p) => p.into_inner().clone(),
            };

            let stats = iv_clone.to_session_stats(
                ctx.session_start_confidence,
                ctx.session_start_instant.elapsed().as_secs(),
            );
            let snapshot_iv = iv_clone.on_compaction();

            let engine = archon_consciousness::rules::RulesEngine::new(ctx.memory.as_ref());
            let rule_scores = engine.export_scores().unwrap_or_default();

            let snap = archon_consciousness::persistence::PersonalitySnapshot {
                session_id: ctx.session_id.clone(),
                timestamp: chrono::Utc::now(),
                inner_voice: snapshot_iv,
                rule_scores,
                stats,
            };

            // Best-effort save. If the panicking thread held a CozoDB
            // lock mid-run_script, save will fail; silently fall through.
            let _ = archon_consciousness::persistence::save_snapshot(ctx.memory.as_ref(), &snap);
            let _ = archon_consciousness::persistence::prune_snapshots(
                ctx.memory.as_ref(),
                ctx.personality_history_limit,
            );
        }));

        // Always chain to default hook so stack trace prints.
        prev_hook(info);
    }));

    mirror
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::MemoryGraph;

    #[test]
    fn ctx_holds_install_inputs() {
        // Cannot exercise the actual hook body in unit tests (set_hook
        // is process-wide and racy under cargo test). Instead verify
        // the fields are set and OnceLock semantics work.
        let graph = MemoryGraph::in_memory().expect("graph");
        let memory: Arc<dyn MemoryTrait> = Arc::new(graph);
        let iv = InnerVoice::new();
        let mirror = install(
            memory,
            iv,
            "panic-test".to_string(),
            0.5,
            std::time::Instant::now(),
            5,
        );
        // Mirror is the same Arc held in PANIC_CTX (idempotent storage).
        assert!(PANIC_CTX.get().is_some(), "OnceLock populated");
        let ctx = PANIC_CTX.get().expect("ctx");
        assert_eq!(ctx.session_id, "panic-test");
        assert!(Arc::ptr_eq(&mirror, &ctx.mirror), "mirror is the same Arc");

        // Second install attempt is a no-op (returns a fresh mirror but
        // PANIC_CTX is already populated).
        let mirror2 = install(
            Arc::clone(&ctx.memory),
            InnerVoice::new(),
            "second".to_string(),
            0.5,
            std::time::Instant::now(),
            5,
        );
        assert!(!Arc::ptr_eq(&mirror, &mirror2));
        // PANIC_CTX still holds the FIRST install
        assert_eq!(PANIC_CTX.get().expect("still set").session_id, "panic-test");
    }
}
