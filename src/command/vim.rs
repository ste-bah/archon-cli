//! TASK-AGS-POST-6-BODIES-B05-VIM: /vim slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Reference: src/command/slash.rs:407 (shipped `/vim` match body —
//!   arm deletion is Gate 3 scope)
//! Based on: src/command/fast.rs (B01-FAST DIRECT precedent — emit-only
//!   sync handler returning `Ok(())` via `ctx.tui_tx.try_send`).
//! Source: Shipped stub `declare_handler!(VimHandler, "Toggle vim-
//!   style modal input")` at registry.rs:728 is REPLACED by the impl
//!   in this file + the matching `insert_primary("vim", ...)` flip at
//!   registry.rs:812 (which now imports `crate::command::vim::VimHandler`).
//!
//! # Why DIRECT (no snapshot, no effect slot)
//!
//! Shipped body at slash.rs:407 performs two sequential `.send().await`
//! calls on `tui_tx`: `VimToggle` (which the TUI interprets as the
//! canonical toggle signal) and a `TextDelta` with the persist-hint
//! message. Both emissions become sync `try_send` calls on
//! `ctx.tui_tx` — the standard DIRECT-pattern replacement (matches
//! B01-FAST, B02-THINKING, B03-BUG precedent). Dropped messages under
//! 16-cap channel backpressure are preferable to stalling the
//! dispatcher; `/vim` output is best-effort informational UI.
//!
//! No state mutation happens in the handler (the TUI owns the vim-mode
//! bool, reacting to `VimToggle`); no new `CommandContext` field is
//! required. Simpler than B01-FAST (no shared atomic) and B04-DIFF (no
//! effect slot).
//!
//! # Byte-for-byte output preservation
//!
//! - `TuiEvent::VimToggle` — enum variant, no string payload.
//! - `TuiEvent::TextDelta` payload preserved verbatim:
//!   `"\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n"`
//!
//! # Trailing-args policy
//!
//! Shipped arm matches exactly `"/vim"`; `/vim on` would have fallen
//! through to the default "unknown command" handler. Post-migration,
//! all `/vim*` inputs route to `VimHandler` via the registry. Chosen
//! strategy: **ignore trailing args and always emit both events** —
//! mirrors the B03-BUG / B04-DIFF trailing-args promotion. Simpler
//! code, better UX.
//!
//! # Aliases
//!
//! Shipped pre-B05-VIM: none. Spec lists none. No aliases added.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/vim` command.
///
/// No aliases. Shipped pre-B05-VIM stub carried none; spec lists
/// none.
pub(crate) struct VimHandler;

impl CommandHandler for VimHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // DIRECT pattern: two sequential sync `try_send` emissions
        // replace the shipped `.send().await` pair at slash.rs:408-412.
        //
        // 1. Canonical toggle signal — TUI owns the vim-mode bool and
        //    flips it on receipt of VimToggle.
        ctx.emit(TuiEvent::VimToggle);

        // 2. Persist-hint TextDelta — byte-identical to the shipped
        //    literal at slash.rs:411.
        ctx.emit(TuiEvent::TextDelta(
            "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n"
                .to_string(),
        ));

        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle vim-style modal input"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    /// Build a minimal CommandContext for /vim tests. /vim is pure
    /// emit-only — no new CommandContext field required, so reuse the
    /// existing `make_status_ctx` helper with a `None` snapshot
    /// (VimHandler never reads status_snapshot).
    fn make_vim_ctx() -> (
        crate::command::registry::CommandContext,
        tokio::sync::mpsc::Receiver<TuiEvent>,
    ) {
        make_status_ctx(None)
    }

    #[test]
    fn vim_handler_emits_vim_toggle_event() {
        let (mut ctx, mut rx) = make_vim_ctx();
        VimHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert!(
            events.iter().any(|e| matches!(e, TuiEvent::VimToggle)),
            "expected TuiEvent::VimToggle in emitted events, got: {:?}",
            events
        );
    }

    #[test]
    fn vim_handler_emits_text_delta_with_persist_message() {
        let (mut ctx, mut rx) = make_vim_ctx();
        VimHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let expected =
            "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n";
        let matched = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s == expected));
        assert!(
            matched,
            "expected TuiEvent::TextDelta with byte-identical persist-hint \
             payload, got: {:?}",
            events
        );
    }

    #[test]
    fn vim_handler_ignores_trailing_args() {
        let (mut ctx, mut rx) = make_vim_ctx();
        VimHandler.execute(&mut ctx, &[String::from("on")]).unwrap();
        let events = drain_tui_events(&mut rx);
        let has_toggle = events.iter().any(|e| matches!(e, TuiEvent::VimToggle));
        let expected =
            "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n";
        let has_delta = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s == expected));
        assert!(
            has_toggle && has_delta,
            "args=[\"on\"] must emit BOTH VimToggle and byte-identical \
             TextDelta (trailing-args ignored per B03-BUG/B04-DIFF \
             promotion policy), got: {:?}",
            events
        );
    }

    #[test]
    fn vim_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            VimHandler.description(),
            "Toggle vim-style modal input",
            "VimHandler.description() must be byte-identical to the \
             shipped declare_handler! arg at registry.rs:728"
        );
    }

    #[test]
    fn vim_handler_execute_returns_ok() {
        let (mut ctx, _rx) = make_vim_ctx();
        let result = VimHandler.execute(&mut ctx, &[]);
        assert!(
            result.is_ok(),
            "VimHandler.execute must return Ok(()) unconditionally \
             (no Err branch; emit-only handler), got: {:?}",
            result
        );
    }

    // -----------------------------------------------------------------
    // Gate 5 live-smoke: end-to-end via real Dispatcher + default
    // Registry (proves routing: dispatcher -> registry -> VimHandler
    // -> channel emission) for the literal user inputs "/vim" and
    // "/vim on". Mirrors the dispatcher-level harness in
    // `src/command/dispatcher.rs::tests` but exercises the real
    // registered VimHandler (no recording fake).
    // -----------------------------------------------------------------

    #[test]
    fn dispatcher_routes_slash_vim_to_vim_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_vim_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/vim");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/vim\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let expected =
            "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n";
        let has_toggle = events.iter().any(|e| matches!(e, TuiEvent::VimToggle));
        let has_delta = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s == expected));
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_toggle && has_delta && !has_error,
            "end-to-end `/vim` must emit VimToggle AND byte-identical \
             TextDelta and NO Error (i.e. not routed to the unknown-command \
             branch); got: {:?}",
            events
        );
    }

    #[test]
    fn dispatcher_routes_slash_vim_on_to_vim_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_vim_ctx();

        // "/vim on" — pre-migration this fell through to the default
        // "unknown command" handler. Post-migration, trailing args are
        // ignored and BOTH events must be emitted.
        let result = dispatcher.dispatch(&mut ctx, "/vim on");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/vim on\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let expected =
            "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n";
        let has_toggle = events.iter().any(|e| matches!(e, TuiEvent::VimToggle));
        let has_delta = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s == expected));
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_toggle && has_delta && !has_error,
            "end-to-end `/vim on` must emit VimToggle AND byte-identical \
             TextDelta (trailing args ignored per B03-BUG/B04-DIFF promotion) \
             and NO Error; got: {:?}",
            events
        );
    }
}
