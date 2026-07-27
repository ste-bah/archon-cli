//! TASK-AGS-POST-6-BODIES-B02-THINKING: /thinking slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Real `CommandHandler` implementation moved here from the legacy slash
//! dispatcher. The handler stores the display preference and emits one
//! semantic `ThinkingToggle` event; the TUI event handler owns the matching
//! informational transcript text so UI messaging cannot terminate an active
//! model-thinking block.
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! The shared preference is an `Arc<AtomicBool>`, so the handler needs no
//! asynchronous state guard or deferred write-back effect. This matches the
//! existing DIRECT command-handler pattern.
//!
//! # Subcommand parse
//!
//! `args.first().map(|s| s.as_str())` selects the action:
//!
//! | match                    | action                  |
//! |--------------------------|-------------------------|
//! | `Some("on")` or `None`   | enable                  |
//! | `Some("off")`            | disable                 |
//! | `Some("archive")`        | open thinking archive   |
//! | `Some(_)`                | silent no-op            |
//!
//! Bare `/thinking` retains the legacy enable behavior. Unknown arguments
//! leave state unchanged and emit no events.
//!
//! Informational text is rendered by the TUI's `ThinkingToggle` handler,
//! preserving the existing enabled/disabled messages without sending them as
//! model `TextDelta` data. This distinction keeps an in-flight thinking block
//! active when the display preference changes.
//!
//! # try_send vs send().await
//!
//! Emission primitive: `ctx.tui_tx.try_send(..)` (sync) instead of the
//! legacy `tui_tx.send(..).await` (async). Matches every peer migrated
//! handler (AGS-806..819) and B01-FAST. `/thinking` output is
//! best-effort informational UI — dropping a message under 16-cap
//! channel backpressure is preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-B02-THINKING: none. Spec lists none. No aliases added.
//! `"/thinking on"` and `"/thinking off"` are positional-arg variants
//! of the SAME `/thinking` primary, NOT aliases — they share the
//! ThinkingHandler dispatch path.

use std::sync::atomic::Ordering;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/thinking` command.
///
/// No aliases. Shipped pre-B02-THINKING stub carried none; spec lists
/// none. The `on`/`off`/empty subcommands are positional args dispatched
/// via the same primary, not separate handlers.
pub(crate) struct ThinkingHandler;

impl CommandHandler for ThinkingHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // 1. Require show_thinking handle. `build_command_context`
        //    populates this unconditionally from
        //    `SlashCommandContext::show_thinking` so at the real
        //    dispatch site this branch never fires. Test fixtures that
        //    construct `CommandContext` directly with
        //    `show_thinking: None` will hit this branch and observe an
        //    Err — mirroring the B01-FAST `fast_mode_shared` and
        //    AGS-815/817 DIRECT-pattern missing-shared-state precedent.
        let shared = ctx.show_thinking.as_ref().ok_or_else(|| {
            anyhow::anyhow!("ThinkingHandler: show_thinking not populated in CommandContext")
        })?;

        // 2. Parse the subcommand. `args.first()` selects on/off/empty;
        //    anything else is a silent no-op (preserves legacy
        //    fall-through at shipped slash.rs:75-90 — `/thinking foo`
        //    matched neither arm).
        let enable = match args.first().map(|s| s.as_str()) {
            // `Some("on")` and `None` (empty args) BOTH enable —
            // mirrors the legacy `"/thinking on" | "/thinking"` arm
            // at slash.rs:75.
            Some("on") | None => true,
            // `Some("off")` mirrors the legacy `"/thinking off"` arm
            // at slash.rs:83.
            Some("off") => false,
            Some("archive") => {
                ctx.emit(TuiEvent::OpenThinkingArchive);
                return Ok(());
            }
            // Unknown arg: silent return. NO state change, NO events,
            // NO error message. Preserves legacy fall-through at
            // shipped slash.rs:75-90 (any non-on/off arg fell through
            // to the default slash handler with no output).
            Some(_) => return Ok(()),
        };

        // 3. DIRECT pattern: sync atomic store. Preserves observable
        //    behavior of the shipped body (which performed the same
        //    atomic store via `ctx.show_thinking.store(bool,
        //    Ordering::Relaxed)`).
        shared.store(enable, Ordering::Relaxed);

        // 4. Emit one semantic event. The TUI event handler updates the
        //    renderer flag and appends informational transcript text without
        //    routing that UI message through model TextDelta semantics.
        ctx.emit(TuiEvent::ThinkingToggle(enable));

        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle extended thinking display on/off"
    }
}

#[cfg(test)]
mod tests {
    // Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
    // skeleton with real assertions against the landed ThinkingHandler
    // impl and the new `CommandContext::show_thinking` field. Uses the
    // `make_thinking_ctx` helper added to `test_support.rs` in this
    // gate.
    //
    // Emission invariant: enable/disable each produce exactly one semantic
    // ThinkingToggle event. Informational transcript text belongs to the TUI
    // event handler, not this command handler.

    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;
    use std::sync::atomic::Ordering;

    #[test]
    fn thinking_handler_on_enables_and_emits_events() {
        let (mut ctx, mut rx) = make_thinking_ctx(false);
        ThinkingHandler
            .execute(&mut ctx, &[String::from("on")])
            .unwrap();
        let shared = ctx.show_thinking.as_ref().unwrap();
        assert!(
            shared.load(Ordering::Relaxed),
            "args=[\"on\"] must transition show_thinking false -> true \
             (mirrors legacy slash.rs:76 atomic store)"
        );

        let events = drain_tui_events(&mut rx);
        assert!(
            matches!(events.as_slice(), [TuiEvent::ThinkingToggle(true)]),
            "thinking enable must emit one semantic toggle event; got: {:?}",
            events
        );
    }

    #[test]
    fn thinking_handler_off_disables_and_emits_events() {
        let (mut ctx, mut rx) = make_thinking_ctx(true);
        ThinkingHandler
            .execute(&mut ctx, &[String::from("off")])
            .unwrap();
        let shared = ctx.show_thinking.as_ref().unwrap();
        assert!(
            !shared.load(Ordering::Relaxed),
            "args=[\"off\"] must transition show_thinking true -> false \
             (mirrors legacy slash.rs:84 atomic store)"
        );

        let events = drain_tui_events(&mut rx);
        assert!(
            matches!(events.as_slice(), [TuiEvent::ThinkingToggle(false)]),
            "thinking disable must emit one semantic toggle event; got: {:?}",
            events
        );
    }

    #[test]
    fn thinking_handler_empty_args_defaults_to_enable() {
        let (mut ctx, mut rx) = make_thinking_ctx(false);
        ThinkingHandler.execute(&mut ctx, &[]).unwrap();
        let shared = ctx.show_thinking.as_ref().unwrap();
        assert!(
            shared.load(Ordering::Relaxed),
            "args=[] (empty) must default to enable — preserves legacy \
             `\"/thinking on\" | \"/thinking\"` arm semantics at \
             slash.rs:75 where the bare `/thinking` invocation enables"
        );

        let events = drain_tui_events(&mut rx);
        assert!(
            matches!(events.as_slice(), [TuiEvent::ThinkingToggle(true)]),
            "bare thinking command must emit one semantic enable event; got: {:?}",
            events
        );
    }

    #[test]
    fn thinking_handler_archive_opens_the_archive_without_changing_display_state() {
        let (mut ctx, mut rx) = make_thinking_ctx(false);

        ThinkingHandler
            .execute(&mut ctx, &[String::from("archive")])
            .unwrap();

        assert!(
            !ctx.show_thinking
                .as_ref()
                .expect("fixture supplies show_thinking")
                .load(Ordering::Relaxed),
            "archive must not alter the thinking display preference"
        );
        assert!(
            matches!(rx.try_recv(), Ok(TuiEvent::OpenThinkingArchive)),
            "archive must emit the archive-open event"
        );
        assert!(
            rx.try_recv().is_err(),
            "archive emits no informational text"
        );
    }

    #[test]
    fn thinking_handler_unknown_arg_is_silent_noop() {
        let (mut ctx, mut rx) = make_thinking_ctx(false);
        let initial = ctx.show_thinking.as_ref().unwrap().load(Ordering::Relaxed);
        ThinkingHandler
            .execute(&mut ctx, &[String::from("foo")])
            .unwrap();
        let shared = ctx.show_thinking.as_ref().unwrap();
        assert_eq!(
            shared.load(Ordering::Relaxed),
            initial,
            "unknown arg must leave show_thinking UNCHANGED — preserves \
             legacy fall-through at slash.rs:75-90 where any non-on/off \
             arg matched no arm and produced no state change"
        );

        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "unknown arg must emit ZERO TuiEvents — preserves legacy \
             fall-through silent semantics; got: {:?}",
            events
        );
    }
}
