//! GHOST-007: /cancel slash-command handler — Option B (feedback + real
//! cancellation).
//!
//! # R1 PURPOSE (real /cancel via slash command)
//!
//! The handler mirrors the Ctrl+C cancel path at
//! `session_loop/mod.rs:406-413`: fire the CancellationToken via
//! `AgentHandle::fire_cancel()` (so the agent stops at its next `.await`)
//! then abort the tracked `JoinHandle` via
//! `AgentDispatcher::cancel_current()` (returns `CancelOutcome`).
//!
//! # R2 Plumbing (GHOST-007)
//!
//! `AgentDispatcher` (for `is_busy` + `cancel_current`) and a late-init slot
//! holding `Arc<AgentHandle>` (for `fire_cancel`) are threaded onto
//! `CommandContext` via `context.rs` → `SlashCommandContext` → `session.rs`.
//! The session loop populates the late-init slot after creating the adapter.
//!
//! # R3 ALIASES `&["stop", "abort"]` (preserved)
//!
//! Shipped-wins per AGS-817 drift-reconcile.

use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;

/// Handler for `/cancel` (aliases: `/stop`, `/abort`).
///
/// GHOST-007 Option B: checks `AgentDispatcher::is_busy()`. If idle, emits
/// "No task is currently running." If busy, fires the cancel token (mirroring
/// Ctrl+C) then calls `cancel_current()` and reports the `CancelOutcome`.
pub(crate) struct CancelHandler;

impl CancelHandler {
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
        // GHOST-007 Option B: real cancellation via AgentDispatcher +
        // AgentHandle. In test contexts where the dispatcher is not
        // wired (ctx.agent_dispatcher is None), fall through to the
        // safe idle message — same behaviour as the old Option A path.
        let disp = match ctx.agent_dispatcher.as_ref() {
            Some(d) => d,
            None => {
                ctx.emit(TuiEvent::TextDelta(
                    "No task is currently running.\n".to_string(),
                ));
                return Ok(());
            }
        };

        // Check busy state under the dispatcher lock.
        if !disp.lock().unwrap().is_busy() {
            ctx.emit(TuiEvent::TextDelta(
                "No task is currently running.\n".to_string(),
            ));
            return Ok(());
        }

        // Fire cancel token first (mirrors Ctrl+C path ordering:
        // adapter.fire_cancel() before dispatcher.cancel_current()).
        if let Some(ref slot) = ctx.cancel_handle
            && let Some(ref handle) = *slot.lock().unwrap()
        {
            handle.fire_cancel();
        }

        // Abort the JoinHandle and report the outcome.
        match disp.lock().unwrap().cancel_current() {
            archon_tui::CancelOutcome::Aborted { elapsed_ms } => {
                ctx.emit(TuiEvent::TextDelta(format!(
                    "Turn cancelled (elapsed: {elapsed_ms}ms).\n"
                )));
            }
            archon_tui::CancelOutcome::NoInflight => {
                // TOCTOU: turn completed between is_busy() and cancel_current().
                ctx.emit(TuiEvent::TextDelta(
                    "No task is currently running.\n".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Cancel the currently running task"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["stop", "abort"]
    }
}

// ---------------------------------------------------------------------------
// GHOST-007: tests for /cancel real cancellation.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    /// Build a minimal `CommandContext` for cancel handler tests.
    /// `agent_dispatcher` and `cancel_handle` default to `None` in
    /// `CtxBuilder`, so the handler emits the idle message (same as
    /// the pre-GHOST-007 Option A behaviour).
    fn make_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn cancel_handler_description_mentions_cancel() {
        // GHOST-007 wired real Option B cancellation. Description now
        // simply says "Cancel the currently running task" — no longer
        // hedges about idle state or Ctrl+C only.
        let h = CancelHandler::new();
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("cancel"),
            "CancelHandler description must mention 'cancel', got: {}",
            h.description()
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
    fn cancel_handler_execute_emits_no_task_running_when_dispatcher_none() {
        // dispatcher = None (test default) → idle message.
        let (mut ctx, mut rx) = make_ctx();
        let h = CancelHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "CancelHandler::execute must return Ok(()), got: {res:?}"
        );
        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(msg)) => {
                assert_eq!(
                    msg, "No task is currently running.\n",
                    "CancelHandler with dispatcher=None must emit idle message"
                );
            }
            Ok(other) => panic!("CancelHandler must emit TextDelta, got: {other:?}"),
            Err(e) => panic!("CancelHandler must emit a TuiEvent, channel returned: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_cancel_emits_feedback() {
        let mut b = RegistryBuilder::new();
        b.insert_primary("cancel", Arc::new(CancelHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/cancel");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/cancel\") must return Ok(()), got: {res:?}"
        );
        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(msg)) => assert_eq!(
                msg, "No task is currently running.\n",
                "Dispatcher route to CancelHandler must emit idle message"
            ),
            Ok(other) => panic!("Dispatcher route must emit TextDelta, got: {other:?}"),
            Err(e) => panic!("Dispatcher route must emit a TuiEvent, channel returned: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_alias_stop_emits_feedback() {
        let mut b = RegistryBuilder::new();
        b.insert_primary("cancel", Arc::new(CancelHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/stop");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/stop\") must return Ok(()), got: {res:?}"
        );
        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(msg)) => assert_eq!(
                msg, "No task is currently running.\n",
                "Dispatcher route via /stop alias must emit idle message"
            ),
            Ok(other) => {
                panic!("Dispatcher route via /stop alias must emit TextDelta, got: {other:?}")
            }
            Err(e) => panic!(
                "Dispatcher route via /stop alias must emit a TuiEvent, channel returned: {e:?}"
            ),
        }
    }
}
