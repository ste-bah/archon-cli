//! TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: /clear slash-command
//! handler (THIN-WRAPPER pattern body-migrate).
//!
//! # R1 DISPATCH-ORDERING (design)
//!
//! The real `/clear` body lives at `src/session.rs:2257-...`, NOT in
//! `src/command/slash.rs`. The TUI input loop intercepts `/clear` with
//! a literal match and runs the body BEFORE `handle_slash_command` is
//! ever invoked, because the body needs `agent.lock().await` to call
//! `Agent::clear_conversation()` plus read/write the personality
//! snapshot + inner-voice state — dependencies that `CommandContext`
//! does not carry and that the sync `CommandHandler::execute` signature
//! cannot `.await`.
//!
//! Dispatch order under normal operation:
//!
//! ```text
//! session.rs:2257  --> /clear intercepted --> `continue` (handler never reached)
//! session.rs:2483  --> handle_slash_command(...)
//! slash.rs         --> Dispatcher::dispatch --> Registry::get("clear")
//! clear.rs         --> ClearHandler::execute    [UNREACHABLE under normal op]
//! ```
//!
//! The `/cls` alias reaches this handler via the PATH A dispatcher
//! because session.rs:2257 only matches the literal `/clear` string.
//! A user typing `/cls` therefore bypasses the shipped clear body
//! today and falls through to this no-op — the same situation that
//! applies to AGS-818 `/export`'s `/save` alias. Real body-migrate
//! will close that gap.
//!
//! # R2 SCOPE-HELD (real body-migrate deferred)
//!
//! Real body-migrate is deferred to POST-STAGE-6. Completing it
//! requires surfacing `Arc<Mutex<Agent>>` plus the personality-persist
//! machinery (`PersonalitySnapshot`, `RulesEngine`, `session_start_*`
//! timers) through `CommandContext`, plus removing the
//! session.rs:2257 interception block without regressing shipped clear
//! behavior.
//!
//! # R3 NO-OP (behavior)
//!
//! This handler is a BYTE-IDENTICAL functional replacement for the
//! shipped `declare_handler!(ClearHandler, "Clear the current
//! conversation", &["cls"])` stub at registry.rs:1208 (pre-B24). The
//! macro-generated body is `Ok(())` with zero emissions. The migrated
//! handler must preserve that EXACT observable behavior — no
//! TextDelta, no Error event, no tui_tx interaction at all. Adding a
//! canary (AGS-818 /export style) would REGRESS observable behavior
//! because the shipped stub emitted nothing.
//!
//! The sentinel at `src/command/slash.rs:98` (`"/compact" | "/clear"
//! => true`) exists so that when the input processor intercept fires
//! and the dispatcher never actually sees the command, the legacy
//! match block still claims recognition (preventing the Option-3
//! default arm and skill-registry double-fire). That sentinel is the
//! parent task's Gate-5 deletion target, NOT this module's concern.
//!
//! # R4 ALIASES `&["cls"]` (preserved)
//!
//! Shipped stub used the three-arg `declare_handler!` form with
//! `&["cls"]`. Per AGS-817 `/memory` (`&["mem"]`) and AGS-818
//! `/export` (`&["save"]`) shipped-wins precedent, this handler
//! preserves the `&["cls"]` alias. Dropping it would regress any
//! operator workflow depending on `/cls` today. The alias resolves
//! through the PATH A dispatcher's alias map (see `Registry::get`).

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/clear` command.
///
/// Alias: `["cls"]` — PRESERVED from the shipped declare_handler!
/// stub (shipped-wins drift-reconcile; see R4 in module rustdoc).
///
/// Under normal operation this handler is UNREACHABLE via `/clear`
/// because `src/session.rs:2257` intercepts first with `continue`.
/// The `/cls` alias reaches the handler through the PATH A dispatcher
/// (session.rs does not match `/cls`). In either case the behavior is
/// the shipped no-op `Ok(())` — byte-identical to the pre-B24
/// `declare_handler!` stub.
pub(crate) struct ClearHandler;

impl ClearHandler {
    /// Construct a fresh `ClearHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for ClearHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for ClearHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // THIN-WRAPPER no-op. Byte-identical to the shipped
        // `declare_handler!` macro body (registry.rs:1163-1180): return
        // Ok(()) WITHOUT emitting any TuiEvent. Real body-migrate
        // deferred to POST-STAGE-6 (see module rustdoc R2).
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1208 (shipped-wins drift-reconcile).
        "Clear the current conversation"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub used the three-arg declare_handler! form with
        // `&["cls"]`. Preserved per AGS-817 /memory and AGS-818
        // /export shipped-wins precedent (see module rustdoc R4).
        &["cls"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: tests for /clear
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    /// Build a minimal `CommandContext` with a freshly-created channel.
    /// /clear is a THIN-WRAPPER handler — no snapshot, no effect
    /// slot, no extra context field — so every optional field stays
    /// `None`. Mirrors the `make_ctx` fixtures in export.rs / compact.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn clear_handler_description_byte_identical_to_shipped() {
        let h = ClearHandler::new();
        assert_eq!(
            h.description(),
            "Clear the current conversation",
            "ClearHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn clear_handler_aliases_match_shipped() {
        let h = ClearHandler::new();
        assert_eq!(
            h.aliases(),
            &["cls"],
            "ClearHandler aliases must preserve 'cls' from the shipped \
             declare_handler! stub (shipped-wins drift-reconcile per \
             AGS-817 /memory and AGS-818 /export precedent)"
        );
    }

    #[test]
    fn execute_returns_ok_without_emission() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ClearHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ClearHandler::execute must return Ok(()), got: {res:?}"
        );
        // No TuiEvent must be emitted — THIN-WRAPPER is byte-identical
        // to the shipped declare_handler! no-op stub.
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "ClearHandler::execute must NOT emit any TuiEvent, \
                 got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_clear_returns_ok_without_emission() {
        // Narrow `RegistryBuilder::new()` (not `default_registry`) so
        // this test exercises ONLY the ClearHandler wiring — no other
        // handlers are registered. Asserts the real Dispatcher routes
        // `/clear` to `ClearHandler::execute` and emits no event.
        let mut b = RegistryBuilder::new();
        b.insert_primary("clear", Arc::new(ClearHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/clear");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/clear\") must return Ok(()), \
             got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route to ClearHandler must NOT emit any \
                 TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_alias_cls_returns_ok_without_emission() {
        // Verify the `&["cls"]` alias resolves to ClearHandler through
        // the Registry's alias map (TASK-AGS-802). This pins the
        // shipped-wins alias-preservation invariant: `/cls` must reach
        // ClearHandler::execute and return Ok(()) with no emission —
        // byte-identical to `/clear`.
        let mut b = RegistryBuilder::new();
        b.insert_primary("clear", Arc::new(ClearHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/cls");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/cls\") must return Ok(()) via \
             the ClearHandler alias, got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route via /cls alias to ClearHandler must \
                 NOT emit any TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }
}
