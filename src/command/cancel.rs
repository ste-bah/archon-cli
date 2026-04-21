//! TASK-AGS-POST-6-NO-STUB: /cancel slash-command handler
//! (THIN-WRAPPER pattern, declare_handler!-macro eliminator).
//!
//! # R1 PURPOSE (THIN-WRAPPER elimination of declare_handler!)
//!
//! This module exists solely to delete the final two `declare_handler!`
//! macro invocations in `src/command/registry.rs` (ConfigHandler at
//! registry.rs:1346-1350 and CancelHandler at registry.rs:1570-1574)
//! and the macro definition itself (registry.rs:1187-1224). Together
//! with `src/command/config.rs`'s ConfigHandler extension, this module
//! lets us remove the `macro_rules! declare_handler` entirely.
//!
//! Mirrors the THIN-WRAPPER precedent set by `src/command/compact.rs`
//! and `src/command/clear.rs` (TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR):
//! a zero-sized struct with a sync `execute` body that returns `Ok(())`
//! WITHOUT emitting any `TuiEvent`. Byte-identical to the shipped
//! `declare_handler!(CancelHandler, "Cancel the currently running
//! task", &["stop", "abort"])` stub at registry.rs:1570-1574.
//!
//! # R2 NO-OP (behavior) + follow-up ticket #91 POST-6-CANCEL-AUDIT
//!
//! Under normal operation /cancel today is a silent no-op: the handler
//! returns `Ok(())` with zero emissions, providing no user feedback
//! about whether a task was actually cancelled. That is a known UX gap
//! but is INTENTIONALLY out of scope for POST-6-NO-STUB — fixing the
//! silent-no-op requires surfacing the task service / cancellation
//! channel through `CommandContext` and writing a real "Cancel
//! requested / no task running" feedback UX. That work is tracked by
//! ticket #91 POST-6-CANCEL-AUDIT. This handler is the byte-identical
//! wrapper around the prior macro stub; it does NOT fix #91, only
//! preserves the exact same observable behavior so the macro can be
//! deleted without regressing shipped semantics.
//!
//! # R3 ALIASES `&["stop", "abort"]` (preserved)
//!
//! Shipped stub used the three-arg `declare_handler!` form with
//! `&["stop", "abort"]`. Per AGS-817 shipped-wins drift-reconcile,
//! this handler preserves both aliases verbatim. Dropping either would
//! regress operator workflows depending on `/stop` or `/abort` today.
//! The aliases resolve through the PATH A dispatcher's alias map
//! (see `Registry::get`).

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/cancel` command.
///
/// Aliases: `["stop", "abort"]` — PRESERVED from the shipped
/// declare_handler! stub (shipped-wins drift-reconcile; see R3 in
/// module rustdoc).
///
/// Behavior is the shipped no-op `Ok(())` — byte-identical to the
/// pre-POST-6-NO-STUB `declare_handler!` macro body. The silent-no-op
/// UX gap is tracked by ticket #91 POST-6-CANCEL-AUDIT and is NOT
/// fixed here.
pub(crate) struct CancelHandler;

impl CancelHandler {
    /// Construct a fresh `CancelHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for CancelHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for CancelHandler {
    fn execute(
        &self,
        _ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // THIN-WRAPPER no-op. Byte-identical to the shipped
        // `declare_handler!` macro body: return `Ok(())` WITHOUT
        // emitting any TuiEvent. The silent-no-op UX gap (no "Cancel
        // requested" / "No task running" feedback) is INTENTIONALLY
        // preserved — fixing it is ticket #91 POST-6-CANCEL-AUDIT.
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1570-1574 (shipped-wins drift-reconcile).
        "Cancel the currently running task"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub used the three-arg declare_handler! form with
        // `&["stop", "abort"]`. Preserved per AGS-817 shipped-wins
        // precedent (see module rustdoc R3).
        &["stop", "abort"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-NO-STUB: tests for /cancel
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
    /// /cancel is a THIN-WRAPPER handler — no snapshot, no effect slot,
    /// no extra context field — so every optional field stays `None`.
    /// Mirrors the `make_ctx` fixtures in compact.rs / clear.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn cancel_handler_description_byte_identical_to_shipped() {
        let h = CancelHandler::new();
        assert_eq!(
            h.description(),
            "Cancel the currently running task",
            "CancelHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn cancel_handler_aliases_match_shipped() {
        let h = CancelHandler::new();
        assert_eq!(
            h.aliases(),
            &["stop", "abort"],
            "CancelHandler aliases must preserve ['stop', 'abort'] from \
             the shipped declare_handler! stub (shipped-wins drift-\
             reconcile per AGS-817 /memory and AGS-818 /export precedent)"
        );
    }

    #[test]
    fn cancel_handler_execute_returns_ok_without_emission() {
        let (mut ctx, mut rx) = make_ctx();
        let h = CancelHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "CancelHandler::execute must return Ok(()), got: {res:?}"
        );
        // No TuiEvent must be emitted — THIN-WRAPPER is byte-identical
        // to the shipped declare_handler! no-op stub. The silent-no-op
        // UX gap is tracked by ticket #91 POST-6-CANCEL-AUDIT.
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "CancelHandler::execute must NOT emit any TuiEvent, \
                 got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_cancel_returns_ok_without_emission() {
        // Narrow `RegistryBuilder::new()` (not `default_registry`) so
        // this test exercises ONLY the CancelHandler wiring — no other
        // handlers are registered. Asserts the real Dispatcher routes
        // `/cancel` to `CancelHandler::execute` and emits no event.
        let mut b = RegistryBuilder::new();
        b.insert_primary("cancel", Arc::new(CancelHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/cancel");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/cancel\") must return Ok(()), \
             got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route to CancelHandler must NOT emit any \
                 TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_alias_stop_returns_ok_without_emission() {
        // Verify the `stop` alias resolves to CancelHandler through
        // the Registry's alias map (TASK-AGS-802). This pins the
        // shipped-wins alias-preservation invariant: `/stop` must
        // reach CancelHandler::execute and return Ok(()) with no
        // emission — byte-identical to `/cancel`.
        let mut b = RegistryBuilder::new();
        b.insert_primary("cancel", Arc::new(CancelHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/stop");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/stop\") must return Ok(()) via \
             the CancelHandler alias, got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route via /stop alias to CancelHandler \
                 must NOT emit any TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }
}
