//! TASK-TUI-621 /teleport slash-command handler (Gate 1 skeleton).
//!
//! Hidden stub command that returns a single "reserved for future use"
//! TextDelta when invoked directly, but is NOT listed in the TUI
//! autocomplete (`crates/archon-tui/src/commands.rs::all_commands()`)
//! so it does not appear in `/help` or the command picker.
//!
//! # Reconciliation with TASK-TUI-621.md spec
//!
//! Spec references `crates/archon-tui/src/slash/teleport.rs` +
//! `SlashCommand` trait + `SlashOutcome::Message`. None of these exist
//! in the actual Archon CLI architecture:
//!
//! - Slash handlers live in the bin crate at `src/command/<name>.rs`,
//!   not in the `archon-tui` library crate.
//! - The trait is `CommandHandler` (re-exported as `SlashCommand` at
//!   `src/command/mod.rs:86` via the TASK-AGS-800 alias shim).
//! - Output is via `ctx.emit(TuiEvent::TextDelta(...))`, not a
//!   `SlashOutcome::Message` enum.
//!
//! The spec's `is_visible() -> false` method does NOT exist on the
//! shipped `CommandHandler` trait. Hiding is implemented by:
//!   (a) registering the handler in `default_registry()` so `/teleport`
//!       IS dispatchable when typed explicitly, AND
//!   (b) OMITTING the entry from `archon-tui::commands::all_commands()`
//!       so the autocomplete / command picker never surfaces it.
//!
//! This matches the Claude Code project-zero `isHidden: true` behavior
//! (hidden from lists, dispatchable when typed).
//!
//! # Gate 1 skeleton
//!
//! This file is intentionally incomplete. Gate 2 (delegated to `coder`
//! subagent) will:
//!   - Replace the `todo!()` in `execute` with the TextDelta emission.
//!   - Remove the `#[ignore]` attributes on the tests and implement the
//!     real assertions using `drain_tui_events` per `BugHandler` pattern.
//!   - Register `TeleportHandler` in `src/command/registry.rs::default_registry()`.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Hidden stub handler for `/teleport`. Dispatchable but not listed.
pub(crate) struct TeleportHandler;

impl CommandHandler for TeleportHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // Gate 2: single TextDelta emission. Leading + trailing `\n`
        // wrap matches the BugHandler pattern at src/command/bug.rs:103.
        // Trailing args intentionally ignored — `_args` underscored.
        ctx.emit(TuiEvent::TextDelta(
            "\nTeleport is reserved for a future release.\n".to_string(),
        ));
        Ok(())
    }

    fn description(&self) -> &str {
        // Used only by `/help teleport` explicit lookup. Not shown in
        // the general `/help` list because the command is absent from
        // `archon-tui::commands::all_commands()`.
        "Reserved for future use"
    }
}

#[cfg(test)]
mod tests {
    // Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
    // skeleton with real assertions against the landed TeleportHandler
    // impl. Uses `make_bug_ctx` / `drain_tui_events` helpers from
    // `test_support.rs` (pattern mirrored from `bug.rs` tests).

    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    #[test]
    fn teleport_returns_reserved_message() {
        let (mut ctx, mut rx) = make_bug_ctx();
        TeleportHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one event; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("reserved"),
                    "expected 'reserved' substring in TextDelta; got: {:?}",
                    s
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got: {:?}", other),
        }
    }

    #[test]
    fn teleport_description_contains_reserved() {
        assert!(
            TeleportHandler
                .description()
                .to_lowercase()
                .contains("reserved"),
            "description() must contain 'reserved' (case-insensitive); \
             got: {:?}",
            TeleportHandler.description()
        );
    }

    #[test]
    fn teleport_absent_from_autocomplete_list() {
        let all = archon_tui::commands::all_commands();
        assert!(
            !all.iter().any(|c| c.name == "/teleport"),
            "/teleport must NOT appear in autocomplete list — hidden via \
             omission from all_commands() per TASK-TUI-621 reconciliation \
             (spec's is_visible() method does not exist on CommandHandler)"
        );
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch, run via --ignored"]
    fn teleport_dispatches_via_registry() {
        // Gate 5 smoke: `Registry::get("teleport")` must return Some(handler)
        // because default_registry() registers TeleportHandler via
        // b.insert_primary("teleport", ...) at registry.rs ~1694.
        // Executing that handler must emit a single TextDelta with
        // "reserved" (case-insensitive). Direct handler construction is
        // intentionally NOT used here — this test proves the registration
        // wiring works, which Gate 4's direct-construction tests do not.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("teleport")
            .expect("teleport must be registered in default_registry() — hidden but dispatchable");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[])
            .expect("dispatched handler must not error");

        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "registry-dispatched /teleport must emit exactly one event; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("reserved"),
                    "dispatched TextDelta must contain 'reserved' (case-insensitive); got: {:?}",
                    s
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta from dispatched /teleport, got: {:?}",
                other
            ),
        }
    }
}
