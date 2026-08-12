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
//! - Panics inside guarded Cozo operations skip snapshot persistence to avoid
//!   re-entering Cozo while its write coordination is active.
//! - Stack-overflow panics may not have stack to run the hook at all.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, OnceLock};

use archon_consciousness::inner_voice::InnerVoice;
use archon_memory::access::MemoryTrait;

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

pub(crate) fn should_capture_panic() -> bool {
    !archon_cozo::in_guarded_operation()
}

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
        if !should_capture_panic() {
            prev_hook(info);
            return;
        }

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
            // Restore the terminal through the SAME helper `TerminalGuard`'s
            // Drop uses so the post-panic console is usable. Calling the
            // shared helper rather than re-listing the sequence here is what
            // makes the keyboard-enhancement pop (issue #174) reach this
            // path: a hand-rolled copy would silently miss it and leave the
            // terminal in modified-keys mode after a panic.
            archon_tui::terminal::restore_terminal();

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

            // Best-effort save for panics outside guarded Cozo operations. Guarded
            // panics bypass this body above to avoid re-entering Cozo.
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
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DELEGATION_CHILD_ENV: &str = "ARCHON_PANIC_SAVE_DELEGATION_CHILD";

    #[test]
    fn guarded_panics_still_reach_the_previous_hook() {
        if std::env::var_os(DELEGATION_CHILD_ENV).is_some() {
            let hook_calls = Arc::new(AtomicUsize::new(0));
            archon_cozo::run_guarded(
                "install Cozo hook",
                cozo::ScriptMutability::Immutable,
                &archon_cozo::CozoGuardConfig::default(),
                || Ok(()),
            )
            .unwrap();

            let cozo_hook = std::panic::take_hook();
            let hook_calls_for_hook = Arc::clone(&hook_calls);
            std::panic::set_hook(Box::new(move |info| {
                hook_calls_for_hook.fetch_add(1, Ordering::SeqCst);
                cozo_hook(info);
            }));

            let graph = MemoryGraph::in_memory().expect("graph");
            let memory: Arc<dyn MemoryTrait> = Arc::new(graph);
            install(
                memory,
                InnerVoice::new(),
                "panic-delegation-test".into(),
                0.5,
                std::time::Instant::now(),
                1,
            );

            let result = archon_cozo::run_guarded(
                "guarded panic",
                cozo::ScriptMutability::Immutable,
                &archon_cozo::CozoGuardConfig::default(),
                || -> anyhow::Result<()> { panic!("guarded panic") },
            );
            assert!(result.is_err());
            assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
            return;
        }

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("panic_save::tests::guarded_panics_still_reach_the_previous_hook")
            .arg("--nocapture")
            .env(DELEGATION_CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success(), "panic-save delegation child failed");
    }

    #[test]
    fn guarded_cozo_operations_disable_personality_capture() {
        let result = archon_cozo::run_guarded(
            "panic-save guard test",
            cozo::ScriptMutability::Immutable,
            &archon_cozo::CozoGuardConfig::default(),
            || -> anyhow::Result<()> {
                assert!(!super::should_capture_panic());
                Ok(())
            },
        );
        result.unwrap();
        assert!(super::should_capture_panic());
    }

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
