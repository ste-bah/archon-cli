//! TASK-AGS-POST-6-BODIES-B01-FAST: /fast slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub at `src/command/registry.rs:546` and the legacy match arm at
//! `src/command/slash.rs:60-70`. The legacy body's `FastModeState`
//! helper is no longer threaded through the handler — the
//! `Arc<AtomicBool>` shared atomic (already owned by
//! `SlashCommandContext::fast_mode_shared`) is the single source of
//! truth, and the handler toggles it directly via load/invert/store.
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! The shipped `/fast` body performed:
//!   1. `fast_mode.toggle()` — sync (inverts + stores on a local
//!      `FastModeState`, returns the new bool).
//!   2. `ctx.fast_mode_shared.store(new_state, Ordering::Relaxed)` —
//!      sync atomic write.
//!   3. `tui_tx.send(TuiEvent::TextDelta(...)).await` — emission only.
//!
//! Step (1) is redundant with step (2): both end up writing
//! `new_state` to the shared atomic (the local `FastModeState` is
//! discarded after the match arm). The body-migrate collapses them
//! into a single load/invert/store on the shared atomic, preserving
//! observable behavior. Consequently:
//!
//! - NO `FastSnapshot` type (nothing to pre-compute inside an async
//!   guard — reads are sync atomic loads).
//! - NO `CommandEffect` variant (the mutation is a sync atomic store,
//!   not a write-back through `tokio::sync::Mutex`).
//! - A new `CommandContext::fast_mode_shared: Option<Arc<AtomicBool>>`
//!   field populated UNCONDITIONALLY by `build_command_context`,
//!   mirroring the AGS-815 `session_id` and AGS-817 `memory`
//!   cross-cutting precedent.
//!
//! The sole side effect besides the atomic store is
//! `ctx.tui_tx.try_send(TuiEvent::TextDelta(..))` — sync and legal
//! inside `CommandHandler::execute`. Matches AGS-810/815/817
//! DIRECT-pattern precedent.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted slash.rs:60-70
//! body:
//! - ENABLED -> `"Fast mode ENABLED. Responses will be faster but lower quality."`
//! - DISABLED -> `"Fast mode DISABLED. Back to normal quality."`
//! - Emission wrapper: `TuiEvent::TextDelta(format!("\n{msg}\n"))`
//!
//! The one emission-primitive change is `tui_tx.send(..).await`
//! (async) -> `ctx.tui_tx.try_send(..)` (sync), matching every peer
//! migrated handler (AGS-806..819). `/fast` output is best-effort
//! informational UI — dropping a message under 16-cap channel
//! backpressure is preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-B01-FAST: none. Spec lists none. No aliases added.

use std::sync::atomic::Ordering;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/fast` command.
///
/// No aliases. Shipped pre-B01-FAST stub carried none; spec lists
/// none.
pub(crate) struct FastHandler;

impl CommandHandler for FastHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // 1. Require fast_mode_shared handle. `build_command_context`
        //    populates this unconditionally from
        //    `SlashCommandContext::fast_mode_shared` so at the real
        //    dispatch site this branch never fires. Test fixtures that
        //    construct `CommandContext` directly with
        //    `fast_mode_shared: None` will hit this branch and observe
        //    an Err — mirroring the AGS-815/817 DIRECT-pattern
        //    missing-shared-state precedent.
        let shared = ctx.fast_mode_shared.as_ref().ok_or_else(|| {
            anyhow::anyhow!("FastHandler: fast_mode_shared not populated in CommandContext")
        })?;

        // 2. DIRECT pattern: sync atomic toggle. Load prev, invert,
        //    store new. Preserves observable behavior of the shipped
        //    body (which performed the same effective transition via
        //    a redundant `FastModeState::toggle` + atomic store).
        let prev = shared.load(Ordering::Relaxed);
        let new_state = !prev;
        shared.store(new_state, Ordering::Relaxed);

        // 3. Byte-for-byte preserved format strings from slash.rs:63-67.
        let msg = if new_state {
            "Fast mode ENABLED. Responses will be faster but lower quality."
        } else {
            "Fast mode DISABLED. Back to normal quality."
        };
        ctx.emit(TuiEvent::TextDelta(format!("\n{msg}\n")));
        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle fast mode (lower quality, faster responses)"
    }
}

#[cfg(test)]
mod tests {
    // Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
    // skeleton with real assertions against the landed FastHandler impl
    // and the new `CommandContext::fast_mode_shared` field. Uses the
    // `make_fast_ctx` helper added to `test_support.rs` in this gate.

    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;
    use std::sync::atomic::Ordering;

    #[test]
    fn fast_handler_toggle_enables_when_initial_disabled() {
        let (mut ctx, mut rx) = make_fast_ctx(false);
        FastHandler.execute(&mut ctx, &[]).unwrap();
        let shared = ctx.fast_mode_shared.as_ref().unwrap();
        assert!(
            shared.load(Ordering::Relaxed),
            "fast_mode_shared must transition false -> true after one \
             FastHandler::execute call"
        );
        let events = drain_tui_events(&mut rx);
        let matched = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Fast mode ENABLED")));
        assert!(
            matched,
            "expected TuiEvent::TextDelta containing 'Fast mode ENABLED', \
             got: {:?}",
            events
        );
    }

    #[test]
    fn fast_handler_toggle_disables_when_initial_enabled() {
        let (mut ctx, mut rx) = make_fast_ctx(true);
        FastHandler.execute(&mut ctx, &[]).unwrap();
        let shared = ctx.fast_mode_shared.as_ref().unwrap();
        assert!(
            !shared.load(Ordering::Relaxed),
            "fast_mode_shared must transition true -> false after one \
             FastHandler::execute call"
        );
        let events = drain_tui_events(&mut rx);
        let matched = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Fast mode DISABLED")));
        assert!(
            matched,
            "expected TuiEvent::TextDelta containing 'Fast mode DISABLED', \
             got: {:?}",
            events
        );
    }

    #[test]
    fn fast_handler_second_invocation_returns_opposite_state() {
        let (mut ctx, mut rx) = make_fast_ctx(false);
        // First call: false -> true.
        FastHandler.execute(&mut ctx, &[]).unwrap();
        let shared = ctx.fast_mode_shared.as_ref().unwrap();
        assert!(
            shared.load(Ordering::Relaxed),
            "after first toggle the shared atomic must be true"
        );
        // Second call: true -> false (round-trip back to initial).
        FastHandler.execute(&mut ctx, &[]).unwrap();
        let shared = ctx.fast_mode_shared.as_ref().unwrap();
        assert!(
            !shared.load(Ordering::Relaxed),
            "after second toggle the shared atomic must return to false \
             (toggle idempotence over two calls: A -> !A -> A)"
        );
        // Both events emitted: ENABLED then DISABLED.
        let events = drain_tui_events(&mut rx);
        let enabled = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Fast mode ENABLED")));
        let disabled = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Fast mode DISABLED")));
        assert!(
            enabled && disabled,
            "expected both 'Fast mode ENABLED' and 'Fast mode DISABLED' \
             TextDelta events across the two toggles, got: {:?}",
            events
        );
    }
}
