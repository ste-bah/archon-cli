//! TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES: /release-notes slash-command
//! handler (Option C, DIRECT pattern body-migrate — static-text emit).
//!
//! Reference: src/command/slash.rs:447-466 (shipped `/release-notes`
//!   match arm body — arm deletion is Gate 5 scope, NOT Gate 2).
//! Based on: src/command/vim.rs (B05-VIM DIRECT precedent — emit-only
//!   sync handler returning `Ok(())` via `ctx.tui_tx.try_send`). The
//!   `/release-notes` body is even simpler than `/vim` (single
//!   `TextDelta` emission vs. `VimToggle` + `TextDelta` pair).
//! Source: Shipped stub
//!   `declare_handler!(ReleaseNotesHandler, "Show release notes for the current build")`
//!   at registry.rs:787 is REPLACED by the impl in this file + the
//!   matching `insert_primary("release-notes", ...)` flip at
//!   registry.rs:869 (which now imports
//!   `crate::command::release_notes::ReleaseNotesHandler`).
//!
//! # Why DIRECT (no snapshot, no effect slot, no new field)
//!
//! Shipped body at slash.rs:447-466 performs exactly one
//! `.send().await` call on `tui_tx` carrying a `TuiEvent::TextDelta`
//! with a static string literal. No state reads, no state mutation,
//! no cross-cutting shared handle. The body-migrate collapses to a
//! single sync `ctx.tui_tx.try_send` call — the standard DIRECT
//! pattern (mirrors B05-VIM but even simpler: no VimToggle partner,
//! no new `CommandContext` field, no trait-level aliases).
//!
//! Dropped messages under 16-cap channel backpressure are preferable
//! to stalling the dispatcher; `/release-notes` output is best-effort
//! informational UI.
//!
//! # Byte-for-byte output preservation
//!
//! The `TextDelta` payload is reproduced byte-identical to the string
//! literal at slash.rs:451-461. The shipped literal uses Rust's `\`
//! line-continuation escape to fold a multi-line source block into a
//! single logical string — each `\` at end-of-line elides both the
//! source newline AND the leading whitespace on the next line. The
//! resulting bytes:
//!
//! 1. `\n`
//! 2. `Archon CLI v0.1.0 (Phase 3)\n\n`
//! 3. nine bullet lines, each `- <text>\n`
//! 4. `\n` separator
//! 5. `Full changelog: https://github.com/archon-cli/archon/releases\n`
//!
//! The handler body and the test-level `EXPECTED_RELEASE_NOTES_BODY`
//! constant both use the identical `\`-continuation idiom so
//! `assert_eq!` with the shipped payload produces a byte-identical
//! match under both source-level grep equivalence and compiled-string
//! equivalence.
//!
//! # Trailing-args policy
//!
//! Shipped arm matches exactly `"/release-notes"`; `/release-notes
//! foo` would have fallen through to the default "unknown command"
//! handler. Post-migration, all `/release-notes*` inputs route to
//! `ReleaseNotesHandler` via the registry. Chosen strategy: **ignore
//! trailing args and always emit the static body** — mirrors the
//! B03-BUG / B04-DIFF / B05-VIM trailing-args promotion. Simpler
//! code, better UX.
//!
//! # Aliases
//!
//! Shipped pre-B07-RELEASE-NOTES: none. Spec lists none. No aliases
//! added.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/release-notes`
/// command.
///
/// No aliases. Shipped pre-B07-RELEASE-NOTES stub carried none; spec
/// lists none.
pub(crate) struct ReleaseNotesHandler;

impl CommandHandler for ReleaseNotesHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // DIRECT pattern: single sync `try_send` emission replaces the
        // shipped `.send().await` at slash.rs:449-464.
        //
        // Static-text TextDelta — byte-identical to the shipped literal
        // at slash.rs:451-461. The `\`-continuation idiom is reproduced
        // verbatim so both source-level `grep` equivalence and
        // compiled-string equivalence hold.
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
            "\nArchon CLI v0.1.0 (Phase 3)\n\n\
         - 33 tasks implemented across 7 batches\n\
         - TUI with markdown rendering, syntax highlighting, vim mode\n\
         - MCP stdio + HTTP transports with lifecycle management\n\
         - Memory graph with HNSW vector search\n\
         - 46 slash commands, hook system, config hot-reload\n\
         - Background sessions, task tools, worktree support\n\
         - Permission model with 6 modes\n\
         - Print mode (-p) for scripting\n\
         - /btw side questions with parallel API calls\n\n\
         Full changelog: https://github.com/archon-cli/archon/releases\n"
                .to_string(),
        ));

        Ok(())
    }

    fn description(&self) -> &str {
        "Show release notes for the current build"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    /// Byte-identical expected TextDelta payload — reproduces the
    /// shipped string literal at slash.rs:451-461 using the same
    /// `\`-continuation idiom. `assert_eq!` against this constant
    /// proves the handler emits the shipped bytes without drift.
    const EXPECTED_RELEASE_NOTES_BODY: &str =
        "\nArchon CLI v0.1.0 (Phase 3)\n\n\
         - 33 tasks implemented across 7 batches\n\
         - TUI with markdown rendering, syntax highlighting, vim mode\n\
         - MCP stdio + HTTP transports with lifecycle management\n\
         - Memory graph with HNSW vector search\n\
         - 46 slash commands, hook system, config hot-reload\n\
         - Background sessions, task tools, worktree support\n\
         - Permission model with 6 modes\n\
         - Print mode (-p) for scripting\n\
         - /btw side questions with parallel API calls\n\n\
         Full changelog: https://github.com/archon-cli/archon/releases\n";

    /// Build a minimal CommandContext for /release-notes tests.
    /// /release-notes is pure emit-only — no new CommandContext field
    /// required, so reuse the existing `make_status_ctx` helper with a
    /// `None` snapshot (ReleaseNotesHandler never reads
    /// status_snapshot).
    fn make_release_notes_ctx() -> (
        crate::command::registry::CommandContext,
        tokio::sync::mpsc::Receiver<TuiEvent>,
    ) {
        make_status_ctx(None)
    }

    #[test]
    fn release_notes_handler_emits_text_delta_with_static_body() {
        let (mut ctx, mut rx) = make_release_notes_ctx();
        ReleaseNotesHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "ReleaseNotesHandler must emit exactly one TuiEvent, got: {:?}",
            events
        );
        let is_text_delta = matches!(events[0], TuiEvent::TextDelta(_));
        assert!(
            is_text_delta,
            "emitted event must be TuiEvent::TextDelta, got: {:?}",
            events[0]
        );
        if let TuiEvent::TextDelta(ref body) = events[0] {
            assert!(
                !body.is_empty(),
                "TextDelta body must be non-empty"
            );
        }
    }

    #[test]
    fn release_notes_handler_body_byte_identical_to_shipped() {
        let (mut ctx, mut rx) = make_release_notes_ctx();
        ReleaseNotesHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let matched = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == EXPECTED_RELEASE_NOTES_BODY)
        });
        assert!(
            matched,
            "expected TuiEvent::TextDelta with byte-identical release-notes \
             payload (from slash.rs:451-461), got: {:?}",
            events
        );
    }

    #[test]
    fn release_notes_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            ReleaseNotesHandler.description(),
            "Show release notes for the current build",
            "ReleaseNotesHandler.description() must be byte-identical to \
             the shipped declare_handler! arg at registry.rs:787"
        );
    }

    #[test]
    fn release_notes_handler_execute_returns_ok() {
        let (mut ctx, _rx) = make_release_notes_ctx();
        let result = ReleaseNotesHandler.execute(&mut ctx, &[]);
        assert!(
            result.is_ok(),
            "ReleaseNotesHandler.execute must return Ok(()) unconditionally \
             (no Err branch; emit-only handler), got: {:?}",
            result
        );
    }

    // -----------------------------------------------------------------
    // Gate 5 live-smoke: end-to-end via real Dispatcher + default
    // Registry (proves routing: dispatcher -> registry ->
    // ReleaseNotesHandler -> channel emission) for literal user input
    // "/release-notes" and the trailing-args promotion case
    // "/release-notes foo". Mirrors the B05-VIM / B06-HELP dispatcher-
    // integration harness but exercises the real registered handler.
    // -----------------------------------------------------------------

    #[test]
    fn dispatcher_routes_slash_release_notes_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_release_notes_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/release-notes");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/release-notes\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == EXPECTED_RELEASE_NOTES_BODY)
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/release-notes` must emit byte-identical TextDelta \
             AND NO Error (i.e. not routed to the unknown-command branch); \
             got: {:?}",
            events
        );
    }

    #[test]
    fn dispatcher_routes_slash_release_notes_with_trailing_args_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_release_notes_ctx();

        // Trailing-args policy: `/release-notes foo` ignores `foo` and
        // emits the static body (mirrors B03/B04/B05/B06 promotion).
        // Pre-migration this would have fallen through to
        // unknown-command; post-migration it routes to the handler.
        let result = dispatcher.dispatch(&mut ctx, "/release-notes foo");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/release-notes foo\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == EXPECTED_RELEASE_NOTES_BODY)
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/release-notes foo` must emit byte-identical \
             TextDelta (trailing-args ignored) AND NO Error; got: {:?}",
            events
        );
    }
}
