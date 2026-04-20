//! TASK-AGS-POST-6-BODIES-B09-COLOR: /color slash-command handler
//! (body-migrate, DIRECT pattern — mirrors AGS-819 /theme precedent).
//!
//! # R1 PATTERN-CONFIRM (DIRECT chosen)
//!
//! The only helper this handler needs in `archon_tui::theme` is sync:
//!
//! * `pub fn parse_color(name: &str) -> Option<ratatui::style::Color>`
//!   (crates/archon-tui/src/theme.rs:15)
//!
//! There is no `.await` anywhere in the shipped body beyond the three
//! `tui_tx.send(..).await` emissions (which R5 swaps for `try_send`).
//! So the handler body has no async surface and the DIRECT pattern
//! applies, mirroring AGS-819 `/theme` verbatim. Specifically:
//!
//! * NO `ColorSnapshot` type (nothing to pre-compute inside an async
//!   guard — `parse_color` is a pure sync match on the arg string).
//! * NO new `CommandContext` field (AGS-822 Rule 5 respected — /color
//!   does not need session_id, memory, shared atomics, or working_dir).
//! * NO `CommandEffect` variant (the accent-color side effect is
//!   emitted directly as `TuiEvent::SetAccentColor(Color)` to the TUI
//!   event loop, which owns the accent-color-mutation responsibility —
//!   there is no `SlashCommandContext` field to write back). Mirrors
//!   AGS-819 /theme's use of `TuiEvent::SetTheme(name)` as the canonical
//!   theme-mutation channel.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `color` is already a primary in the default registry via the
//! `declare_handler!(ColorHandler, "Show or change the UI color scheme")`
//! stub at registry.rs:874 (no aliases). This ticket is a body-migrate,
//! NOT a gap-fix: primary count is UNCHANGED. The stub is REMOVED in
//! favour of the real type defined in this file, imported into
//! registry.rs at the top via `use crate::command::color::ColorHandler;`.
//!
//! # R3 NO-ALIASES (shipped-wins drift-reconcile)
//!
//! Shipped `declare_handler!` stub at registry.rs:874 carried no alias
//! slice — equivalent to `&[]`. AGS-817 /memory established the
//! shipped-wins drift-reconcile rule: zero aliases shipped → zero
//! aliases preserved. This handler returns `&[]` from `aliases()` and
//! the test `color_handler_aliases_are_empty` pins the invariant against
//! silent additions.
//!
//! # R4 ARGS-RECONCILIATION
//!
//! The shipped body in slash.rs:693-713 used
//! `s.strip_prefix("/color").unwrap_or("").trim()` on the raw input
//! string — a single-string substring after the command name. The
//! parser tokenizes on whitespace into `args: &[String]`. For every
//! current color name (`red`, `green`, `yellow`, `blue`, `magenta`,
//! `cyan`, `white`, `default`, plus the `purple`/`reset`/`none`
//! synonyms `parse_color` accepts — all single-word), `args.first()`
//! is byte-equivalent to the shipped semantics. To defend against
//! future spec drift that introduces multi-word color names (e.g.
//! `"bright red"`), this handler uses `args.join(" ").trim()` which
//! gracefully degrades to the same single-token form for the current
//! name set AND preserves the shipped substring semantics for any
//! future multi-word name. Empty args (bare `/color`) and a
//! whitespace-only join both map to the help-branch, matching the
//! shipped `if color_arg.is_empty()` check. Mirrors AGS-819 /theme R4.
//!
//! # R5 EMISSION-PRIMITIVE-SWAP (.await -> try_send)
//!
//! Shipped body emitted via `tui_tx.send(..).await` — async, blocking
//! on backpressure if the 16-cap channel is full. The sync
//! `CommandHandler::execute` signature cannot `.await`, so this handler
//! uses `ctx.tui_tx.try_send(..)` (sync, best-effort drop on full).
//! Matches AGS-806..819 emission precedent verbatim. The dropped event
//! semantics are acceptable for an informational overlay: a full TUI
//! channel implies the operator's terminal is already saturated with
//! pending events and dropping an accent-color-confirmation TextDelta
//! does not corrupt state. The `TuiEvent::SetAccentColor` variant — the
//! actual accent-color-mutation signal — is dispatched first via
//! `try_send`; if the channel is full, the color will not change and
//! the operator will see neither confirmation nor mutation, which is
//! the correct unified degraded-mode behavior. All three shipped format
//! strings are preserved BYTE-FOR-BYTE:
//!
//! 1. `"\nAvailable accent colors: red, green, yellow, blue, magenta, \
//!    cyan, white, default\nUsage: /color <name>\n"` (help branch —
//!    note that the shipped source used a `\<newline>[spaces]`
//!    string-continuation idiom; the compiled bytes are a single
//!    two-line string with no embedded indentation).
//! 2. `"\nAccent color set to '{color_arg}'.\n"` (success confirmation).
//! 3. `"Unknown color '{color_arg}'. Available: red, green, yellow, \
//!    blue, magenta, cyan, white, default"` (error).

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/color` command.
///
/// No aliases (see R3 in module rustdoc). Body-migrate of the shipped
/// arm at slash.rs:692-713 — DIRECT pattern (sync `parse_color`, no
/// snapshot, no effect slot, no extra `CommandContext` field).
///
/// # Behavior
///
/// * Empty args (bare `/color`) → emit a help TextDelta listing the
///   available accent colors and a usage hint.
/// * Args naming a known color (matches `parse_color(name).is_some()`)
///   → emit `TuiEvent::SetAccentColor(color)` THEN a confirmation
///   TextDelta.
/// * Args naming an unknown color → emit a `TuiEvent::Error` listing
///   the valid names.
pub(crate) struct ColorHandler;

impl CommandHandler for ColorHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R4: join multi-token args with " " and trim. For all current
        // single-token color names this collapses to the same value as
        // `args.first().unwrap_or("").as_str()`; for any future
        // multi-word name it preserves the shipped substring semantics
        // (`s.strip_prefix("/color").unwrap_or("").trim()`). Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the help branch identical to the shipped
        // `if color_arg.is_empty()` check.
        let joined = args.join(" ");
        let color_arg = joined.trim();

        if color_arg.is_empty() {
            // Help branch — byte-for-byte preservation of shipped
            // format string at slash.rs:696-699 (the shipped source
            // used a `\<newline>[spaces]` string-continuation; the
            // compiled bytes are a single string with no embedded
            // indentation).
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                "\nAvailable accent colors: red, green, yellow, blue, magenta, cyan, white, default\nUsage: /color <name>\n".to_string()
            ));
        } else if let Some(color) = archon_tui::theme::parse_color(color_arg) {
            // Valid color — emit SetAccentColor first (the actual
            // mutation signal), then the confirmation TextDelta. Order
            // matches shipped slash.rs:700-706.
            let _ = ctx.tui_tx.try_send(TuiEvent::SetAccentColor(color));
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                "\nAccent color set to '{color_arg}'.\n"
            )));
        } else {
            // Invalid color — error branch byte-for-byte from shipped
            // slash.rs:707-710.
            let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                "Unknown color '{color_arg}'. Available: red, green, yellow, blue, magenta, cyan, white, default"
            )));
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:874 (shipped-wins drift-reconcile).
        "Show or change the UI color scheme"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // test `color_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B09-COLOR: tests for /color slash-command
// body-migrate. Mirrors the structure of src/command/theme.rs tests
// (AGS-819 precedent) — local `make_ctx` helper inside this module, no
// additions to test_support.rs (DIRECT pattern doesn't need a new
// shared helper).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /color is a DIRECT-pattern handler — no snapshot, no effect
    /// slot, no extra context field — so every optional field stays
    /// `None`. Mirrors the make_ctx fixtures in theme.rs / voice.rs /
    /// export.rs.
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
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    /// The description must match the shipped `declare_handler!` stub
    /// at registry.rs:874 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
    #[test]
    fn color_handler_description_byte_identical_to_shipped() {
        let h = ColorHandler;
        assert_eq!(
            h.description(),
            "Show or change the UI color scheme",
            "ColorHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    /// Shipped `declare_handler!` stub at registry.rs:874 carried no
    /// alias slice — equivalent to `&[]`. AGS-817 shipped-wins rule
    /// preserves zero aliases.
    #[test]
    fn color_handler_aliases_are_empty() {
        let h = ColorHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "ColorHandler must have an empty alias slice per B09 R3 \
             (shipped declare_handler! stub had no aliases)"
        );
    }

    /// Bare `/color` (no args) must emit a `TextDelta` containing the
    /// shipped header `"Available accent colors:"` and the usage hint
    /// `"Usage: /color <name>"`. Additionally pins the byte-for-byte
    /// preservation of the full compiled help string (the shipped
    /// source used a `\<newline>[spaces]` string-continuation idiom).
    #[test]
    fn color_handler_execute_with_no_args_emits_list_and_usage() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ColorHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ColorHandler::execute(no-args) must return Ok(()), got: {res:?}"
        );

        let ev = rx
            .try_recv()
            .expect("ColorHandler::execute(no-args) must emit one event");
        match ev {
            TuiEvent::TextDelta(text) => {
                // Byte-for-byte pin: the shipped compiled help string
                // is a single two-line string with NO embedded
                // indentation (the backslash-continuation in the
                // shipped source elides the newline + leading spaces).
                let expected = "\nAvailable accent colors: red, green, yellow, blue, magenta, cyan, white, default\nUsage: /color <name>\n";
                assert_eq!(
                    text, expected,
                    "no-args branch TextDelta must match shipped help \
                     string byte-for-byte (note: the shipped source used \
                     a `\\<newline>[spaces]` continuation; compiled bytes \
                     have no embedded indentation)"
                );
            }
            other => panic!(
                "no-args branch must emit TextDelta, got: {other:?}"
            ),
        }
    }

    /// A valid color name must emit TWO events in order: `SetAccentColor`
    /// first (the mutation signal), then a confirmation `TextDelta`. The
    /// confirmation text must match the shipped format
    /// `"\nAccent color set to '{name}'.\n"` byte-for-byte. Uses `"cyan"`
    /// as the canonical known-valid color.
    #[test]
    fn color_handler_execute_with_valid_color_emits_setaccent_and_confirmation() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ColorHandler;
        let name = "cyan";
        // Sanity: assert the color parser agrees so the test would
        // catch a regression in the upstream theme module rather than
        // failing mysteriously.
        assert!(
            archon_tui::theme::parse_color(name).is_some(),
            "test premise broken: parse_color('{name}') must return Some"
        );

        let res = h.execute(&mut ctx, &[name.to_string()]);
        assert!(
            res.is_ok(),
            "ColorHandler::execute(valid) must return Ok(()), got: {res:?}"
        );

        // First event MUST be SetAccentColor carrying the exact Color
        // returned by `parse_color("cyan")`. We compare against the
        // parser's output rather than hardcoding `ratatui::style::Color::Cyan`
        // because the bin crate does not depend on `ratatui` directly —
        // the Color type lives in archon_tui's TuiEvent variant and the
        // canonical source of truth is `parse_color` itself.
        let expected_color = archon_tui::theme::parse_color(name)
            .expect("parse_color('cyan') must return Some");
        let first = rx
            .try_recv()
            .expect("valid-color branch must emit at least one event");
        match first {
            TuiEvent::SetAccentColor(payload) => {
                assert_eq!(
                    payload, expected_color,
                    "first event must be SetAccentColor carrying the \
                     parse_color('{name}') output, got: \
                     SetAccentColor({payload:?})"
                );
            }
            other => panic!(
                "valid-color branch first event must be SetAccentColor, \
                 got: {other:?}"
            ),
        }

        // Second event MUST be the confirmation TextDelta with the
        // exact shipped format.
        let second = rx
            .try_recv()
            .expect("valid-color branch must emit a confirmation event");
        match second {
            TuiEvent::TextDelta(text) => {
                let expected = format!("\nAccent color set to '{name}'.\n");
                assert_eq!(
                    text, expected,
                    "valid-color branch confirmation TextDelta must \
                     match shipped format byte-for-byte"
                );
            }
            other => panic!(
                "valid-color branch second event must be TextDelta, \
                 got: {other:?}"
            ),
        }
    }

    /// An unknown color name must emit a single `Error` event with the
    /// shipped format `"Unknown color '{name}'. Available: red, green,
    /// yellow, blue, magenta, cyan, white, default"`.
    #[test]
    fn color_handler_execute_with_unknown_color_emits_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ColorHandler;
        let bogus = "not-a-real-color-xyz";
        // Defensive sanity: confirm the bogus name really is unknown
        // so a future addition to the color parser can't silently make
        // this test pass via the wrong branch.
        assert!(
            archon_tui::theme::parse_color(bogus).is_none(),
            "test premise broken: '{bogus}' must NOT resolve to a color"
        );

        let res = h.execute(&mut ctx, &[bogus.to_string()]);
        assert!(
            res.is_ok(),
            "ColorHandler::execute(unknown) must return Ok(()), got: {res:?}"
        );

        let ev = rx
            .try_recv()
            .expect("unknown-color branch must emit one event");
        match ev {
            TuiEvent::Error(text) => {
                let expected = format!(
                    "Unknown color '{bogus}'. Available: red, green, yellow, blue, magenta, cyan, white, default"
                );
                assert_eq!(
                    text, expected,
                    "unknown-color branch Error must match shipped \
                     format byte-for-byte"
                );
            }
            other => panic!(
                "unknown-color branch must emit Error, got: {other:?}"
            ),
        }
    }

    /// Defensive test for R4: passing a 2-token args slice (spec drift
    /// defence — current colors are all single words but the parser
    /// could in theory feed multi-token args) must not panic. The
    /// handler's join-and-trim logic should:
    /// * Either treat the joined string as an unknown color and emit
    ///   an Error (current behaviour for any 2-word combo since no
    ///   current color name has spaces).
    /// * Or, if a future color parser adds a multi-word name like
    ///   `"bright red"`, treat it as a valid color and emit
    ///   SetAccentColor + confirmation.
    /// Either outcome is acceptable; the only hard requirement is that
    /// `execute` returns `Ok(())` without panic and emits at least one
    /// event.
    #[test]
    fn color_handler_execute_joins_multi_token_args_without_panicking() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ColorHandler;
        let args = vec!["bright".to_string(), "red".to_string()];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "ColorHandler::execute(multi-token) must return Ok(()), \
             got: {res:?}"
        );

        // At least one event must land — either an Error (current
        // behaviour for unknown joined name) or a SetAccentColor/
        // TextDelta pair (future-proof for multi-word color names).
        let mut event_count = 0;
        while rx.try_recv().is_ok() {
            event_count += 1;
        }
        assert!(
            event_count >= 1,
            "ColorHandler::execute(multi-token) must emit at least one \
             event, got: {event_count}"
        );
    }

    // -----------------------------------------------------------------
    // Gate 5 live-smoke: end-to-end via real Dispatcher + default
    // Registry (proves routing: dispatcher -> registry -> ColorHandler
    // -> channel emission) for literal user input "/color" and the
    // trailing-args case "/color cyan". Mirrors B05-VIM / B06-HELP /
    // B07-RELEASE-NOTES / B08-DENIALS dispatcher-integration harness
    // but exercises the real registered handler.
    // -----------------------------------------------------------------

    #[test]
    fn dispatcher_routes_slash_color_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        // Bare "/color" → help branch → TextDelta with byte-identical
        // compiled help string.
        let result = dispatcher.dispatch(&mut ctx, "/color");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/color\") must return Ok"
        );

        let expected_help = "\nAvailable accent colors: red, green, yellow, blue, magenta, cyan, white, default\nUsage: /color <name>\n";
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == expected_help)
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/color` must emit byte-identical help TextDelta \
             AND NO Error (i.e. not routed to the unknown-command \
             branch); got: {:?}",
            events
        );
    }

    #[test]
    fn dispatcher_routes_slash_color_with_trailing_args_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        // "/color cyan" → valid-arg branch → SetAccentColor + TextDelta
        // confirmation. Pre-migration this was a single async body in
        // slash.rs:692-713; post-migration it routes to the handler via
        // dispatcher + parser tokenization ("color" primary + ["cyan"]
        // args). This exercises the arg-consumption path (as opposed to
        // the trailing-args-ignored pattern in B07/B08 where the shipped
        // arm matched exactly on the bare command string).
        let result = dispatcher.dispatch(&mut ctx, "/color cyan");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/color cyan\") must return Ok"
        );

        let expected_confirmation = "\nAccent color set to 'cyan'.\n";
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let has_set_accent = events
            .iter()
            .any(|e| matches!(e, TuiEvent::SetAccentColor(_)));
        let has_text_delta = events.iter().any(|e| {
            matches!(e, TuiEvent::TextDelta(s) if s == expected_confirmation)
        });
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_set_accent && has_text_delta && !has_error,
            "end-to-end `/color cyan` must emit BOTH SetAccentColor AND \
             byte-identical confirmation TextDelta AND NO Error; got: \
             {:?}",
            events
        );
    }
}
