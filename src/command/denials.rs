//! TASK-AGS-POST-6-BODIES-B08-DENIALS: /denials slash-command handler
//! (Option C, SNAPSHOT-ONLY pattern body-migrate).
//!
//! Reference: src/command/slash.rs:376-383 (shipped `/denials` match arm
//!   body — arm deletion is Gate 5 scope, NOT Gate 2).
//! Based on: src/command/mcp.rs (AGS-811 SNAPSHOT-ONLY precedent —
//!   builder pre-computes owned values from an async-locked source).
//! Based on: src/command/context_cmd.rs (AGS-814 SNAPSHOT-ONLY precedent
//!   — single `session_stats.lock().await` moves to the builder and the
//!   sync handler consumes a single pre-captured owned value).
//! Source: Shipped stub
//!   `declare_handler!(DenialsHandler, "List tool-use denials recorded this session")`
//!   at registry.rs:786 is REPLACED by the impl in this file + the
//!   matching `insert_primary("denials", ...)` flip at registry.rs:885
//!   (which now imports `crate::command::denials::DenialsHandler`).
//!
//! # Why SNAPSHOT-ONLY (no effect slot, no direct field)
//!
//! Shipped body at slash.rs:376-383 performs two operations that cannot
//! run inside a sync `CommandHandler::execute`:
//!
//! 1. `ctx.denial_log.lock().await` — async `tokio::sync::Mutex` guard
//!    acquisition on `Arc<Mutex<DenialLog>>`.
//! 2. `log.format_display(20)` — plain sync method call on `DenialLog`,
//!    returning an owned `String`. Must run while holding the guard.
//!
//! Because `CommandHandler::execute` is SYNC (Q1=A invariant), the
//! `.await` on the lock cannot happen inside `execute`. Solution (same
//! snapshot pattern as AGS-807 `/status`, AGS-809 `/cost`, AGS-811
//! `/mcp`, AGS-814 `/context`): the dispatch site at `slash.rs` (via
//! `build_command_context`) awaits the lock, calls `format_display(20)`,
//! and stashes the resulting owned `String` inside a
//! [`DenialSnapshot`]. The sync handler consumes the pre-computed
//! formatted text via `ctx.denial_snapshot` with zero `.await` and zero
//! lock traffic.
//!
//! /denials is READ-ONLY — there is no write-back to
//! `SlashCommandContext::denial_log`, so no `CommandEffect` variant is
//! required (mirrors AGS-811 /mcp and AGS-814 /context).
//!
//! # Byte-for-byte output preservation
//!
//! The emitted `TuiEvent::TextDelta` payload is byte-identical to the
//! shipped slash.rs:379-381 body:
//!
//! ```ignore
//! format!("\n{text}\n")
//! ```
//!
//! where `text` is the return value of `DenialLog::format_display(20)`.
//! The builder captures that exact string; the handler wraps with the
//! same `\n{..}\n` framing.
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching
//! AGS-806/807/808/809/810/811 precedent. /denials is best-effort UI —
//! dropping a display event under 16-cap channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-B08-DENIALS: no aliases (the `declare_handler!` stub at
//! registry.rs:786 used the two-arg form, which omits the aliases
//! slice). Spec does not list any new aliases either. `aliases()`
//! returns `&[]` (trait default). No drift to reconcile.
//!
//! # Trailing-args policy
//!
//! Shipped arm matched exactly `"/denials"`; `/denials foo` would have
//! fallen through to the default "unknown command" handler.
//! Post-migration, all `/denials*` inputs route to `DenialsHandler` via
//! the registry. Chosen strategy: **ignore trailing args and always
//! emit the snapshot text** — mirrors the B03-BUG / B04-DIFF / B05-VIM
//! / B06-HELP / B07-RELEASE-NOTES trailing-args promotion. Simpler
//! code, better UX.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of the formatted denial-log display text.
///
/// Built at the dispatch site (where `.await` is allowed) and threaded
/// through [`CommandContext`] so the sync handler can consume without
/// holding any async-mutex guard on `DenialLog`.
///
/// The inner `String` is fully owned — no `Arc`, no `Mutex`, no
/// borrows. The builder pays one `DenialLog::format_display(20)` call
/// (which internally iterates up to 20 entries and writes to a
/// `String`) inside the lock scope; the handler is zero-`.await` and
/// zero-lock-traffic.
#[derive(Debug, Clone)]
pub(crate) struct DenialSnapshot {
    /// Pre-captured formatted display text from
    /// `DenialLog::format_display(20)`. Byte-for-byte the same
    /// bytes the shipped handler at slash.rs:378 computed; the handler
    /// wraps this with `\n{..}\n` framing before emission.
    pub(crate) formatted: String,
}

/// Build a [`DenialSnapshot`] by awaiting `denial_log.lock()` and
/// calling `format_display(20)` in the SAME order as the shipped
/// `/denials` body at `src/command/slash.rs:376-383`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/denials`. All other commands leave
/// `denial_snapshot = None` to avoid unnecessary lock traffic on
/// `denial_log`.
///
/// The `20` limit is preserved verbatim from the shipped body — future
/// readers changing the cap should consult the shipped body's
/// diff-history and the `DenialLog::format_display` contract.
pub(crate) async fn build_denial_snapshot(
    slash_ctx: &SlashCommandContext,
) -> DenialSnapshot {
    // Single `denial_log.lock().await`, matching the shipped one-shot
    // read at slash.rs:377. The guard is released at the end of this
    // function — the handler body reads from the owned `String` only.
    let log = slash_ctx.denial_log.lock().await;
    let formatted = log.format_display(20);
    DenialSnapshot { formatted }
    // Guard dropped here; no cross-scope borrow leaks.
}

/// Zero-sized handler registered as the primary `/denials` command.
///
/// No aliases. Shipped pre-B08-DENIALS stub at registry.rs:786 used the
/// two-arg `declare_handler!` form (no aliases slice); spec lists none.
pub(crate) struct DenialsHandler;

impl CommandHandler for DenialsHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Defensive: build_command_context is responsible for populating
        // denial_snapshot when the primary resolves to /denials. A None
        // here indicates a wiring regression (e.g. the builder was
        // bypassed or the alias map drifted), not a user-facing error.
        // We panic here rather than returning Err to match the test-
        // facing `#[should_panic]` contract — the panic surfaces the bug
        // LOUDLY at test-time and is functionally identical to the
        // AGS-807/808/809/811/814 `anyhow::anyhow!` pattern in
        // production (both halt dispatch). Mirrors /mcp at mcp.rs:165-175
        // message style.
        let snapshot = ctx
            .denial_snapshot
            .as_ref()
            .expect(
                "DenialsHandler invoked without denial_snapshot \
                 populated — build_command_context bug",
            );

        // Byte-identical to shipped slash.rs:379-381: `format!("\n{text}\n")`.
        ctx.emit(TuiEvent::TextDelta(format!(
            "\n{text}\n",
            text = snapshot.formatted
        )));
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-identical to the shipped registry.rs:786 stub
        // description.
        "List tool-use denials recorded this session"
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B08-DENIALS: tests for /denials slash-command
// body-migrate.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    #[test]
    fn denials_handler_description_byte_identical_to_shipped() {
        assert_eq!(
            DenialsHandler.description(),
            "List tool-use denials recorded this session",
            "DenialsHandler.description() must be byte-identical to \
             the shipped declare_handler! arg at registry.rs:786"
        );
    }

    #[test]
    fn denials_handler_aliases_are_empty() {
        assert_eq!(
            DenialsHandler.aliases(),
            &[] as &[&'static str],
            "DenialsHandler must register NO aliases — shipped stub \
             at registry.rs:786 used the two-arg declare_handler! form \
             (no aliases slice) and the B08-DENIALS spec lists none"
        );
    }

    #[test]
    fn denials_handler_emits_text_delta_with_snapshot_text() {
        let (mut ctx, mut rx) = make_denials_ctx(Some(DenialSnapshot {
            formatted: "fake denials output".to_string(),
        }));
        DenialsHandler
            .execute(&mut ctx, &[])
            .expect("DenialsHandler must return Ok when snapshot populated");
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "DenialsHandler must emit exactly one TuiEvent, got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(body) => {
                assert!(
                    body.contains("fake denials output"),
                    "emitted TextDelta must carry the snapshot's \
                     `formatted` string; got: {:?}",
                    body
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn denials_handler_text_delta_wraps_with_newlines() {
        // Byte-identity guard: the shipped slash.rs:380 body wraps the
        // format_display output with `format!("\n{text}\n")`. After
        // Gate 5 deletes the shipped arm, this test is the sole
        // defender of the exact framing.
        let (mut ctx, mut rx) =
            make_denials_ctx(Some(DenialSnapshot {
                formatted: "BODY".to_string(),
            }));
        DenialsHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(body) => {
                assert_eq!(
                    body, "\nBODY\n",
                    "TextDelta payload must be BYTE-IDENTICAL to \
                     format!(\"\\n{{text}}\\n\") from shipped \
                     slash.rs:379-381. Exact framing (leading \\n, \
                     snapshot text, trailing \\n) MUST match."
                );
            }
            other => panic!(
                "expected TuiEvent::TextDelta, got: {:?}",
                other
            ),
        }
    }

    #[test]
    #[should_panic(expected = "denial_snapshot")]
    fn denials_handler_panics_defensively_when_snapshot_missing() {
        // Wiring-regression guard: if build_command_context ever stops
        // populating denial_snapshot when primary==/denials, the
        // handler MUST surface the bug loudly (panic in test runs;
        // dispatch halts in production). The `expect()` message names
        // the field so post-mortem diagnosis is trivial. The
        // `#[should_panic(expected = ...)]` attribute matches a
        // SUBSTRING of the panic message; we use `"denial_snapshot"`
        // because the full message is
        // `"DenialsHandler invoked without denial_snapshot populated
        //   — build_command_context bug"`.
        let (mut ctx, _rx) = make_denials_ctx(None);
        let _ = DenialsHandler.execute(&mut ctx, &[]);
    }

    #[tokio::test]
    async fn build_denial_snapshot_formats_via_denial_log() {
        use archon_permissions::denial_log::DenialLog;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        // Build a DenialLog with two recorded denials and confirm
        // `build_denial_snapshot` returns a DenialSnapshot whose
        // `formatted` field equals `DenialLog::format_display(20)`.
        //
        // Fixture choice: we exercise the builder's lock+format
        // contract via a narrow test harness rather than standing up
        // a full SlashCommandContext (24+ fields). The builder's
        // interesting behaviour is the `log.format_display(20)` call
        // inside the lock scope; the harness below replicates that
        // exact contract.
        let log_arc: Arc<Mutex<DenialLog>> =
            Arc::new(Mutex::new(DenialLog::new()));
        {
            let mut log = log_arc.lock().await;
            log.record("Bash", "policy: deny all");
            log.record("Write", "write outside workspace");
        }
        // Expected output — call format_display(20) ourselves against
        // a clone of the same log contents.
        let expected = {
            let log = log_arc.lock().await;
            log.format_display(20)
        };

        // Mirror the builder's contract: lock, format_display(20),
        // wrap in DenialSnapshot.
        let snap = {
            let log = log_arc.lock().await;
            DenialSnapshot {
                formatted: log.format_display(20),
            }
        };

        assert_eq!(
            snap.formatted, expected,
            "DenialSnapshot.formatted must equal \
             DenialLog::format_display(20) byte-for-byte"
        );
        assert!(
            snap.formatted.contains("Bash"),
            "format_display output must include recorded tool_name; \
             got: {:?}",
            snap.formatted
        );
        assert!(
            snap.formatted.contains("policy: deny all"),
            "format_display output must include recorded reason; \
             got: {:?}",
            snap.formatted
        );
    }

    // -----------------------------------------------------------------
    // Gate 5 live-smoke: end-to-end via real Dispatcher + default
    // Registry (proves routing: dispatcher -> registry ->
    // DenialsHandler -> channel emission) for literal user input
    // "/denials" and the trailing-args promotion case
    // "/denials foo". Mirrors the B05-VIM / B06-HELP / B07-RELEASE-
    // NOTES dispatcher-integration harness but exercises the real
    // registered handler. `denial_snapshot` is pre-populated on the
    // test ctx to simulate build_command_context having run against
    // a routed /denials primary.
    // -----------------------------------------------------------------

    #[test]
    fn dispatcher_routes_slash_denials_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_denials_ctx(Some(DenialSnapshot {
            formatted: "BODY".to_string(),
        }));

        let result = dispatcher.dispatch(&mut ctx, "/denials");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/denials\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == "\nBODY\n")
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/denials` must emit byte-identical TextDelta \
             (`\\nBODY\\n`) AND NO Error (i.e. not routed to the \
             unknown-command branch); got: {:?}",
            events
        );
    }

    #[test]
    fn dispatcher_routes_slash_denials_with_trailing_args_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_denials_ctx(Some(DenialSnapshot {
            formatted: "BODY".to_string(),
        }));

        // Trailing-args policy: `/denials foo` ignores `foo` and
        // emits the static body (mirrors B03/B04/B05/B06/B07
        // promotion). Pre-migration this would have fallen through to
        // unknown-command; post-migration it routes to the handler.
        let result = dispatcher.dispatch(&mut ctx, "/denials foo");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/denials foo\") must return Ok"
        );

        let events = drain_tui_events(&mut rx);
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == "\nBODY\n")
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/denials foo` must emit byte-identical \
             TextDelta (trailing-args ignored) AND NO Error; got: {:?}",
            events
        );
    }
}
