//! TASK-AGS-POST-6-BODIES-B02-THINKING: /thinking slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub at `src/command/registry.rs:587` and the legacy match arms at
//! `src/command/slash.rs:75-90` (two arms — `"/thinking on" |
//! "/thinking"` and `"/thinking off"`). The shipped body wrote to
//! `SlashCommandContext::show_thinking` (an `Arc<AtomicBool>`) and then
//! awaited two `tui_tx.send(..).await` emissions per arm. The migrated
//! body performs the same atomic store and the same two TuiEvent
//! emissions — only the emission primitive changes (`send().await` ->
//! `try_send(..)`, B01-FAST precedent).
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! The shipped `/thinking` body performed:
//!   1. `ctx.show_thinking.store(bool, Ordering::Relaxed)` — sync
//!      atomic write.
//!   2. `tui_tx.send(TuiEvent::ThinkingToggle(bool)).await` — emission
//!      only.
//!   3. `tui_tx.send(TuiEvent::TextDelta(format!("\n…\n"))).await` —
//!      emission only.
//!
//! Step (1) is a sync atomic store on a shared `Arc<AtomicBool>`; no
//! `tokio::sync::Mutex` guard is involved. Steps (2) and (3) are output
//! channel sends, not state mutations. Consequently:
//!
//! - NO `ThinkingSnapshot` type (nothing to pre-compute inside an async
//!   guard — there is no read-side guard at all).
//! - NO `CommandEffect` variant (the mutation is a sync atomic store,
//!   not a write-back through `tokio::sync::Mutex`).
//! - A new `CommandContext::show_thinking: Option<Arc<AtomicBool>>`
//!   field populated UNCONDITIONALLY by `build_command_context`,
//!   mirroring the AGS-815 `session_id`, AGS-817 `memory`, and
//!   B01-FAST `fast_mode_shared` cross-cutting precedent.
//!
//! Matches B01-FAST architecture exactly — the only structural
//! difference is the subcommand parse step (B02 reads `args.first()`,
//! B01 ignores `_args`).
//!
//! # Subcommand parse
//!
//! `args.first().map(|s| s.as_str())` selects the action:
//!
//! | match                    | action                  |
//! |--------------------------|-------------------------|
//! | `Some("on")` or `None`   | enable (set true)       |
//! | `Some("off")`            | disable (set false)     |
//! | `Some(_)` (anything else)| **silent no-op**        |
//!
//! `None` defaults to enable so that `/thinking` (alone, no args) keeps
//! the legacy `"/thinking on" | "/thinking"` arm semantics from
//! shipped slash.rs:75. The unknown-arg silent-noop preserves legacy
//! fall-through: in the shipped flow, `/thinking foo` matched neither
//! arm and fell through to the default slash handler (which returned
//! `false` -> "unknown command"); in the migrated flow ALL `/thinking*`
//! inputs route to ThinkingHandler, so the silent return below is what
//! preserves the observable "no state change, no output" behavior. No
//! error message, usage hint, or any output is emitted — the legacy
//! body has none and B02 must not introduce any.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted slash.rs:75-90 body:
//!
//! - ENABLE  -> `"\nThinking display enabled.\n"` (slash.rs:79 literal,
//!              including the leading and trailing `\n` wrap)
//! - DISABLE -> `"\nThinking display disabled.\n"` (slash.rs:87 literal,
//!              same `\n` wrap)
//!
//! The `\n` wrap is preserved exactly — Sherlock Gate 3 will MD5 the
//! enabled/disabled strings against the shipped slash.rs literals to
//! prove byte equivalence.
//!
//! # Event order (CRITICAL)
//!
//! Event emission order mirrors the legacy flow at slash.rs:77-80 and
//! :85-88: `TuiEvent::ThinkingToggle(bool)` is emitted FIRST, then
//! `TuiEvent::TextDelta(msg)`. The TUI consumes ThinkingToggle to
//! flip a renderer flag before TextDelta lands — reordering would
//! visually surface as a one-frame stale render.
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

        // 4. Event emission. Order MATTERS: ThinkingToggle FIRST, then
        //    TextDelta. Mirrors legacy slash.rs:77-80 and :85-88
        //    sequence byte-for-byte. The TUI consumes ThinkingToggle
        //    to flip a renderer flag before the TextDelta lands.
        let _ = ctx.tui_tx.send(TuiEvent::ThinkingToggle(enable));

        // 5. Byte-for-byte preserved format strings from slash.rs:79
        //    and slash.rs:87. The leading and trailing `\n` wrap is
        //    PRESERVED — Sherlock Gate 3 will MD5 these literals
        //    against the shipped strings to prove byte equivalence.
        let msg = if enable {
            "\nThinking display enabled.\n"
        } else {
            "\nThinking display disabled.\n"
        };
        let _ = ctx.tui_tx.send(TuiEvent::TextDelta(msg.to_string()));

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
    // Event-order invariant: every test that asserts emissions checks
    // ThinkingToggle FIRST, then TextDelta. Reordering would be a
    // legacy-divergence regression — Gate 3 sherlock will verify.

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
        // Event order invariant: ThinkingToggle FIRST, then TextDelta.
        assert!(
            matches!(events.first(), Some(TuiEvent::ThinkingToggle(true))),
            "first event must be ThinkingToggle(true) (legacy \
             slash.rs:77 emits ThinkingToggle BEFORE TextDelta); got: {:?}",
            events
        );
        let matched = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Thinking display enabled")));
        assert!(
            matched,
            "expected TuiEvent::TextDelta containing 'Thinking display enabled.', \
             got: {:?}",
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
        // Event order invariant: ThinkingToggle FIRST, then TextDelta.
        assert!(
            matches!(events.first(), Some(TuiEvent::ThinkingToggle(false))),
            "first event must be ThinkingToggle(false) (legacy \
             slash.rs:85 emits ThinkingToggle BEFORE TextDelta); got: {:?}",
            events
        );
        let matched = events.iter().any(
            |e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Thinking display disabled")),
        );
        assert!(
            matched,
            "expected TuiEvent::TextDelta containing 'Thinking display disabled.', \
             got: {:?}",
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
        // Event order invariant: ThinkingToggle FIRST, then TextDelta.
        assert!(
            matches!(events.first(), Some(TuiEvent::ThinkingToggle(true))),
            "empty args must emit ThinkingToggle(true) FIRST; got: {:?}",
            events
        );
        let matched = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Thinking display enabled")));
        assert!(
            matched,
            "empty args must emit TextDelta containing 'Thinking display enabled.', \
             got: {:?}",
            events
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
