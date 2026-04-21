//! TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: /compact slash-command
//! handler (THIN-WRAPPER pattern body-migrate).
//!
//! # R1 DISPATCH-ORDERING (design)
//!
//! The real `/compact` body lives at `src/session.rs:2241-2253`, NOT in
//! `src/command/slash.rs`. The TUI input loop intercepts `/compact`
//! (and `/compact <subcommand>`) with a literal prefix match and runs
//! the body BEFORE `handle_slash_command` is ever invoked, because the
//! body needs `agent.lock().await` to call `Agent::compact(subcommand)`
//! — a dependency that `CommandContext` does not carry and that the
//! sync `CommandHandler::execute` signature cannot `.await`.
//!
//! Dispatch order under normal operation:
//!
//! ```text
//! session.rs:2241  --> /compact intercepted --> `continue` (handler never reached)
//! session.rs:2483  --> handle_slash_command(...)
//! slash.rs         --> Dispatcher::dispatch --> Registry::get("compact")
//! compact.rs       --> CompactHandler::execute    [UNREACHABLE under normal op]
//! ```
//!
//! # R2 SCOPE-HELD (real body-migrate deferred)
//!
//! Real body-migrate is deferred to POST-STAGE-6 (same deferral pattern
//! as `/export` at AGS-818 / AGS-POST-6-EXPORT-MIGRATE). Completing it
//! requires surfacing `Arc<Mutex<Agent>>` (or an effect-slot pattern)
//! through `CommandContext` so the handler can call the async
//! `Agent::compact(subcommand).await`, plus removing the
//! session.rs:2241-2253 interception block without regressing shipped
//! compact behavior.
//!
//! # R3 NO-OP (behavior)
//!
//! This handler is a BYTE-IDENTICAL functional replacement for the
//! shipped `declare_handler!(CompactHandler, "Compact the current
//! conversation history")` stub at registry.rs:1207 (pre-B24). The
//! macro-generated body is `Ok(())` with zero emissions. The migrated
//! handler must preserve that EXACT observable behavior — no TextDelta,
//! no Error event, no tui_tx interaction at all. Adding a canary
//! (AGS-818 /export style) would REGRESS observable behavior because
//! the shipped stub emitted nothing.
//!
//! The sentinel at `src/command/slash.rs:98` (`"/compact" | "/clear" =>
//! true`) exists so that when the input processor intercept fires and
//! the dispatcher never actually sees the command, the legacy match
//! block still claims recognition (preventing the Option-3 default arm
//! and skill-registry double-fire). That sentinel is the parent task's
//! Gate-5 deletion target, NOT this module's concern.
//!
//! # R4 ALIASES (none)
//!
//! Shipped stub used the two-arg `declare_handler!` form with no
//! aliases. Per AGS-817 shipped-wins drift-reconcile, this handler also
//! declares `&[]`. No new aliases added.

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/compact` command.
///
/// No aliases. Under normal operation this handler is UNREACHABLE
/// because `src/session.rs:2241` intercepts `/compact` with `continue`
/// before the dispatcher is invoked. If `execute` fires anyway the
/// behavior is the shipped no-op `Ok(())` — byte-identical to the
/// pre-B24 `declare_handler!` stub.
pub(crate) struct CompactHandler;

impl CompactHandler {
    /// Construct a fresh `CompactHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for CompactHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for CompactHandler {
    fn execute(
        &self,
        _ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // THIN-WRAPPER no-op. Byte-identical to the shipped
        // `declare_handler!` macro body (registry.rs:1163-1180): return
        // Ok(()) WITHOUT emitting any TuiEvent. Real body-migrate
        // deferred to POST-STAGE-6 (see module rustdoc R2).
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1207 (shipped-wins drift-reconcile).
        "Compact the current conversation history"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub used the two-arg declare_handler! form — no
        // aliases. AGS-817 shipped-wins drift-reconcile preserves zero
        // aliases (see module rustdoc R4).
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: tests for /compact
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
    /// /compact is a THIN-WRAPPER handler — no snapshot, no effect
    /// slot, no extra context field — so every optional field stays
    /// `None`. Mirrors the `make_ctx` fixtures in export.rs / voice.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                context_snapshot: None,
                session_id: None,
                memory: None,
                garden_config: None,
                fast_mode_shared: None,
                show_thinking: None,
                working_dir: None,
                skill_registry: None,
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                copy_snapshot: None,
                doctor_snapshot: None,
                usage_snapshot: None,
                config_path: None,
                auth_label: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    #[test]
    fn compact_handler_description_byte_identical_to_shipped() {
        let h = CompactHandler::new();
        assert_eq!(
            h.description(),
            "Compact the current conversation history",
            "CompactHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn compact_handler_aliases_match_shipped() {
        let h = CompactHandler::new();
        let empty: &[&str] = &[];
        assert_eq!(
            h.aliases(),
            empty,
            "CompactHandler aliases must be empty — shipped stub used \
             the two-arg declare_handler! form with no aliases"
        );
    }

    #[test]
    fn execute_returns_ok_without_emission() {
        let (mut ctx, mut rx) = make_ctx();
        let h = CompactHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "CompactHandler::execute must return Ok(()), got: {res:?}"
        );
        // No TuiEvent must be emitted — THIN-WRAPPER is byte-identical
        // to the shipped declare_handler! no-op stub.
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "CompactHandler::execute must NOT emit any TuiEvent, \
                 got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatcher_routes_slash_compact_returns_ok_without_emission() {
        // Narrow `RegistryBuilder::new()` (not `default_registry`) so
        // this test exercises ONLY the CompactHandler wiring — no other
        // handlers are registered. Asserts the real Dispatcher routes
        // `/compact` to `CompactHandler::execute` and emits no event.
        let mut b = RegistryBuilder::new();
        b.insert_primary("compact", Arc::new(CompactHandler::new()));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let res = dispatcher.dispatch(&mut ctx, "/compact");
        assert!(
            res.is_ok(),
            "Dispatcher::dispatch(\"/compact\") must return Ok(()), \
             got: {res:?}"
        );
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ev) => panic!(
                "Dispatcher route to CompactHandler must NOT emit any \
                 TuiEvent, got: {ev:?}"
            ),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }
}
