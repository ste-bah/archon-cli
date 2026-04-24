//! TASK-AGS-POST-6-CANCEL-AUDIT: /cancel slash-command handler with
//! user-visible feedback UX (Option A — feedback-only).
//!
//! # R1 PURPOSE (user-visible /cancel feedback)
//!
//! This handler emits a visible `TuiEvent::TextDelta` on every invocation
//! so users who type `/cancel` (or its aliases `/stop` / `/abort`) see
//! explicit feedback instead of the prior silent no-op. The ACTUAL
//! in-flight cancel path is `__cancel__` (Ctrl+C) at session.rs:2126-2138
//! — that pathway calls `adapter.fire_cancel()` and
//! `dispatcher.cancel_current()` to abort the running turn. Because the
//! session input loop processes slash commands serially, by the time a
//! typed `/cancel` reaches this handler the preceding turn has already
//! completed, so we report the idle state and point the user at Ctrl+C
//! for in-flight cancellation.
//!
//! # R2 Option A (feedback-only) — deliberate scope choice
//!
//! This ticket implements Option A per the spec:
//! an unconditional `TuiEvent::TextDelta("No task is currently running.\n")`
//! emission that satisfies AC #1's "either X or Y" requirement by picking
//! one of the two documented strings. Option B (feedback + real
//! cancellation propagation via a SIDECAR-SLOT `Arc<AtomicBool>` plumbed
//! through `CommandContext` into the session turn loop) is deferred —
//! `archon_tui::AgentDispatcher::cancel_current()` CONSUMES the in-flight
//! state rather than probing it, and no sync `is_idle()`/`in_flight()`
//! probe exists on that dispatcher. Surfacing such a probe is a separate
//! ticket if Option B ever ships.
//!
//! # R3 ALIASES `&["stop", "abort"]` (preserved)
//!
//! The shipped stub used the three-arg `declare_handler!` form with
//! `&["stop", "abort"]`. Both aliases are PRESERVED here per AGS-817
//! shipped-wins drift-reconcile — dropping either would regress operator
//! workflows that rely on `/stop` or `/abort`. Aliases resolve through
//! the PATH A dispatcher's alias map (see `Registry::get`).

use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;

/// Zero-sized handler registered as the primary `/cancel` command.
///
/// Aliases: `["stop", "abort"]` — preserved from the shipped
/// `declare_handler!` stub.
///
/// Emits an unconditional `TuiEvent::TextDelta("No task is currently
/// running.\n")` via `CommandContext::emit` (POST-6-TRY-SEND) so users
/// see explicit feedback. For in-flight task cancellation, users press
/// Ctrl+C — the `__cancel__` pathway in session.rs handles that.
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
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // Option A (feedback-only): unconditional user-visible message.
        // The session input loop is serial — by the time this handler
        // runs, the previous turn has completed, so the idle-state
        // message is correct. In-flight cancel is Ctrl+C (__cancel__
        // at session.rs:2126-2138).
        ctx.emit(TuiEvent::TextDelta(
            "No task is currently running.\n".to_string(),
        ));
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // description at registry.rs:1570-1574.
        "Cancel the currently running task"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub used `&["stop", "abort"]`. PRESERVED per AGS-817
        // shipped-wins precedent (see module rustdoc R3).
        &["stop", "abort"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-CANCEL-AUDIT: tests for /cancel user-visible feedback.
// ---------------------------------------------------------------------------
// Prior POST-6-NO-STUB tests asserted ZERO emission; this ticket REPLACES
// them with emission-asserting tests that verify the user-visible message.
// Deleted tests (from pre-CANCEL-AUDIT cancel.rs):
//   - cancel_handler_execute_returns_ok_without_emission
//   - dispatcher_routes_slash_cancel_returns_ok_without_emission
//   - dispatcher_routes_alias_stop_returns_ok_without_emission
// The description + aliases tests are retained verbatim (shipped-wins
// invariants unchanged).
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
    /// /cancel is a feedback-only handler — no snapshot field required.
    fn make_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
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
    fn cancel_handler_execute_emits_no_task_running_feedback() {
        let (mut ctx, mut rx) = make_ctx();
        let h = CancelHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "CancelHandler::execute must return Ok(()), got: {res:?}"
        );
        // POST-6-CANCEL-AUDIT: handler now emits a user-visible
        // TextDelta with the idle-state message (Option A feedback-only).
        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(msg)) => {
                assert_eq!(
                    msg, "No task is currently running.\n",
                    "CancelHandler::execute must emit the Option A \
                     idle-state feedback string verbatim"
                );
            }
            Ok(other) => panic!("CancelHandler::execute must emit TextDelta, got: {other:?}"),
            Err(e) => panic!(
                "CancelHandler::execute must emit a TuiEvent, channel \
                 returned: {e:?}"
            ),
        }
        // No trailing events.
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "CancelHandler::execute emitted exactly one event; \
                 unexpected trailing event: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_cancel_emits_feedback() {
        // Narrow `RegistryBuilder::new()` (not `default_registry`) so
        // this test exercises ONLY the CancelHandler wiring.
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
            Ok(TuiEvent::TextDelta(msg)) => assert_eq!(
                msg, "No task is currently running.\n",
                "Dispatcher route to CancelHandler must emit the Option \
                 A idle-state feedback string verbatim"
            ),
            Ok(other) => panic!("Dispatcher route must emit TextDelta, got: {other:?}"),
            Err(e) => panic!(
                "Dispatcher route must emit a TuiEvent, channel \
                 returned: {e:?}"
            ),
        }
    }

    #[test]
    fn dispatcher_routes_alias_stop_emits_feedback() {
        // Verify `stop` alias resolves to CancelHandler through the
        // Registry's alias map and emits the same feedback string.
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
            Ok(TuiEvent::TextDelta(msg)) => assert_eq!(
                msg, "No task is currently running.\n",
                "Dispatcher route via /stop alias must emit the Option \
                 A idle-state feedback string verbatim"
            ),
            Ok(other) => panic!(
                "Dispatcher route via /stop alias must emit TextDelta, \
                 got: {other:?}"
            ),
            Err(e) => panic!(
                "Dispatcher route via /stop alias must emit a TuiEvent, \
                 channel returned: {e:?}"
            ),
        }
    }
}
