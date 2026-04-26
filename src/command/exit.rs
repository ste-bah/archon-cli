//! TASK-#206 SLASH-EXIT — `/exit` slash-command handler + `q` alias.
//!
//! The graceful-shutdown sequence (SessionEnd hook, personality
//! snapshot, watch-path teardown) runs inline in
//! `src/session_loop/mod.rs` at the top of the slash-dispatch block:
//! the input processor short-circuits `/exit`, `/quit`, and `/q`
//! before the registry dispatcher is invoked. That inline path owns
//! the heavy state (agent handle, persist-personality flag) which is
//! not reachable from a `CommandContext`.
//!
//! This handler exists so:
//!   1. `/exit` is discoverable via the command picker / `/help` —
//!      it now lives in the primary command registry alongside the
//!      other slash commands.
//!   2. `/q` is wired as a real alias on the command registry. The
//!      previous attempt at `src/session.rs:1928` registered the
//!      alias on the SKILL registry pointing to a non-existent
//!      `exit` skill, which was dead code. That line is removed in
//!      the same commit and replaced by the `aliases()` declaration
//!      below.
//!   3. Any future flow that bypasses the inline shortcut (or a
//!      headless-test path) lands on a handler that emits the
//!      canonical shutdown signal (`TuiEvent::Done`) so the TUI
//!      event loop breaks cleanly.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/exit` handler — emits the `TuiEvent::Done` shutdown signal.
pub(crate) struct ExitHandler;

impl CommandHandler for ExitHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        ctx.emit(TuiEvent::Done);
        Ok(())
    }

    fn description(&self) -> &str {
        "Exit the session"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["q"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn execute_emits_done() {
        let handler = ExitHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one Done event; got {:?}",
            events
        );
        match &events[0] {
            TuiEvent::Done => {}
            other => panic!("expected TuiEvent::Done, got {:?}", other),
        }
    }

    #[test]
    fn description_non_empty() {
        assert!(!ExitHandler.description().is_empty());
    }

    #[test]
    fn alias_q_listed() {
        assert_eq!(ExitHandler.aliases(), &["q"]);
    }

    #[test]
    fn execute_ignores_args() {
        // Spurious args must not change the outcome — `/exit foo bar`
        // still produces the canonical shutdown signal.
        let handler = ExitHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[String::from("foo"), String::from("bar")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TuiEvent::Done));
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn exit_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("exit") must return Some(handler) +
        // execute must emit exactly one TuiEvent::Done. Alias "q" must
        // resolve to the same handler.
        use crate::command::registry::default_registry;

        let registry = default_registry();

        // Primary lookup.
        let handler = registry
            .get("exit")
            .expect("exit must be registered in default_registry()");
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one Done event on primary path; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::Done => {}
            other => panic!("expected TuiEvent::Done on primary path, got: {:?}", other),
        }

        // Alias resolution: /q must route to the same handler and emit
        // the same Done event. This is the regression guard against the
        // skill-registry dead-alias bug fixed at src/session.rs:1928.
        let via_alias = registry
            .get("q")
            .expect("alias q must resolve to exit handler");
        let (mut ctx2, mut rx2) = make_bug_ctx();
        via_alias.execute(&mut ctx2, &[]).unwrap();
        let alias_events = drain_tui_events(&mut rx2);
        assert_eq!(
            alias_events.len(),
            1,
            "alias path must emit exactly one Done event; got: {:?}",
            alias_events
        );
        assert!(
            matches!(alias_events[0], TuiEvent::Done),
            "alias path must emit Done, got: {:?}",
            alias_events[0]
        );
    }
}
