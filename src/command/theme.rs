//! TASK-AGS-819: /theme slash-command handler (Option C, body-migrate,
//! DIRECT pattern, FIFTH Batch-3 ticket).
//!
//! # R1 PATTERN-CONFIRM (DIRECT chosen)
//!
//! Every helper this handler needs in `archon_tui::theme` is sync:
//!
//! * `pub fn theme_by_name(name: &str) -> Option<Theme>` (theme.rs:38)
//! * `pub fn available_themes() -> &'static [&'static str]` (theme.rs:70)
//!
//! There is no `.await` in either function body — both are pure
//! match-on-string lookups. So the handler body has no async surface
//! and the DIRECT pattern applies, mirroring AGS-812 `/hooks` and
//! AGS-816 `/voice`. Specifically:
//!
//! * NO `ThemeSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp`).
//! * NO new `CommandContext` field (AGS-822 Rule 5 respected — first
//!   ticket that ACTUALLY needs a context field is the first ticket
//!   that adds it; /theme does not need one).
//! * NO `CommandEffect` variant (the theme-set side effect is emitted
//!   directly as `TuiEvent::SetTheme(name)` to the TUI event loop,
//!   which owns the theme-mutation responsibility — there is no
//!   `SlashCommandContext` field to write back). The forward-looking
//!   `CommandEffect` rustdoc at registry.rs:272 listed AGS-819 as a
//!   speculative "write" extension of the enum; that turned out to be
//!   wrong because `TuiEvent::SetTheme` is the canonical side-effect
//!   channel for theme changes — see registry.rs:272 NOTE explaining
//!   the actual DIRECT pattern.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `theme` is already a primary in the default registry via the
//! `declare_handler!(ThemeHandler, "Show or change the UI theme")` stub
//! at registry.rs:607 (no aliases). This ticket is a body-migrate, NOT
//! a gap-fix: `EXPECTED_COMMAND_COUNT=40` is UNCHANGED. The stub is
//! REMOVED in favour of the real type defined in this file, imported
//! into registry.rs at the top via `use crate::command::theme::ThemeHandler;`.
//!
//! # R3 NO-ALIASES (shipped-wins drift-reconcile)
//!
//! Shipped `declare_handler!` stub at registry.rs:607 carried no alias
//! slice — equivalent to `&[]`. AGS-817 /memory established the
//! shipped-wins drift-reconcile rule: zero aliases shipped → zero
//! aliases preserved (the inverse of preserving `&["mem"]` for /memory
//! and `&["save"]` for /export). This handler returns `&[]` from
//! `aliases()` and the registry test `registry_theme_primary_with_no_aliases`
//! pins the invariant against silent additions.
//!
//! # R4 ARGS-RECONCILIATION
//!
//! The shipped body in slash.rs:754-780 used
//! `s.strip_prefix("/theme").unwrap_or("").trim()` on the raw input
//! string — a single-string substring after the command name. The
//! parser tokenizes on whitespace into `args: &[String]`. For all
//! current single-token theme names (`intj`, `intp`, ..., `dark`,
//! `mono`, etc. — every entry in `available_themes()` is one word with
//! no spaces), `args.first()` is byte-equivalent to the shipped
//! semantics. To defend against future spec drift that introduces
//! multi-word theme names, this handler uses `args.join(" ").trim()`
//! which gracefully degrades to the same single-token form for the
//! current name set AND preserves the shipped substring semantics for
//! any future multi-word name. Empty args (bare `/theme`) and a
//! whitespace-only join both map to the list-mode branch, matching the
//! shipped `if theme_arg.is_empty()` check.
//!
//! # R5 EMISSION-PRIMITIVE-SWAP (.await -> try_send)
//!
//! Shipped body emitted via `tui_tx.send(..).await` — async, blocking
//! on backpressure if the 16-cap channel is full. The sync
//! `CommandHandler::execute` signature cannot `.await`, so this handler
//! uses `ctx.tui_tx.try_send(..)` (sync, best-effort drop on full).
//! Matches AGS-806..818 emission precedent verbatim. The dropped event
//! semantics are acceptable for an informational overlay: a full TUI
//! channel implies the operator's terminal is already saturated with
//! pending events and dropping a theme-confirmation TextDelta does not
//! corrupt state. The `TuiEvent::SetTheme` variant — the actual
//! theme-mutation signal — is dispatched first via `try_send`; if the
//! channel is full, the theme will not change and the operator will
//! see neither confirmation nor mutation, which is the correct unified
//! degraded-mode behavior. All four shipped format strings are
//! preserved BYTE-FOR-BYTE:
//!
//! 1. `"\nAvailable themes: {names}\nUsage: /theme <name>\n"` (list
//!    branch + invalid-arg help reuse).
//! 2. `"\nTheme set to '{theme_arg}'.\n"` (success confirmation).
//! 3. `"Unknown theme '{theme_arg}'. Available: {names}"` (error).

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/theme` command.
///
/// No aliases (see R3 in module rustdoc). Body-migrate of the shipped
/// arm at slash.rs:754-780 — DIRECT pattern (sync theme helpers, no
/// snapshot, no effect slot, no extra `CommandContext` field).
///
/// # Behavior
///
/// * Empty args (bare `/theme`) → list available themes via
///   `available_themes()` joined with `", "` and a usage hint.
/// * Args naming a known theme (matches `theme_by_name(name).is_some()`)
///   → emit `TuiEvent::SetTheme(name)` THEN a confirmation TextDelta.
/// * Args naming an unknown theme → emit a `TuiEvent::Error` listing
///   the valid names.
pub(crate) struct ThemeHandler;

impl CommandHandler for ThemeHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R4: join multi-token args with " " and trim. For all current
        // single-token theme names this collapses to the same value as
        // `args.first().unwrap_or("").as_str()`; for any future
        // multi-word name it preserves the shipped substring semantics
        // (`s.strip_prefix("/theme").unwrap_or("").trim()`). Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the list branch identical to the shipped
        // `if theme_arg.is_empty()` check.
        let joined = args.join(" ");
        let theme_arg = joined.trim();

        if theme_arg.is_empty() {
            // List branch — byte-for-byte preservation of shipped
            // format string at slash.rs:760-762.
            let names = archon_tui::theme::available_themes().join(", ");
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                "\nAvailable themes: {names}\nUsage: /theme <name>\n"
            )));
        } else if archon_tui::theme::theme_by_name(theme_arg).is_some() {
            // Valid theme — emit SetTheme first (the actual mutation
            // signal), then the confirmation TextDelta. Order matches
            // shipped slash.rs:765-770.
            let _ = ctx
                .tui_tx
                .try_send(TuiEvent::SetTheme(theme_arg.to_string()));
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                "\nTheme set to '{theme_arg}'.\n"
            )));
        } else {
            // Invalid theme — error branch byte-for-byte from shipped
            // slash.rs:773-776.
            let names = archon_tui::theme::available_themes().join(", ");
            let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                "Unknown theme '{theme_arg}'. Available: {names}"
            )));
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:607 (shipped-wins drift-reconcile).
        "Show or change the UI theme"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // registry test `registry_theme_primary_with_no_aliases`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-819: tests for /theme slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /theme is a DIRECT-pattern handler — no snapshot, no effect
    /// slot, no extra context field — so every optional field stays
    /// `None`. Mirrors the make_ctx fixtures in voice.rs / export.rs.
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
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /theme tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /theme tests never exercise /diff paths — None.
                working_dir: None,
                // TASK-AGS-POST-6-BODIES-B06-HELP: /theme tests never exercise /help paths — None.
                skill_registry: None,
                // TASK-AGS-POST-6-BODIES-B08-DENIALS: /theme tests never exercise /denials paths — None.
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
    fn theme_handler_description_matches() {
        let h = ThemeHandler;
        assert_eq!(
            h.description(),
            "Show or change the UI theme",
            "ThemeHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn theme_handler_aliases_are_empty() {
        let h = ThemeHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "ThemeHandler must have an empty alias slice per AGS-819 R3 \
             (shipped declare_handler! stub had no aliases)"
        );
    }

    /// Bare `/theme` (no args) must emit a `TextDelta` containing the
    /// shipped header `"Available themes:"` and the usage hint
    /// `"Usage: /theme <name>"`. Pins the byte-for-byte preservation of
    /// the shipped list-branch format string.
    #[test]
    fn theme_handler_execute_with_no_args_emits_list() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ThemeHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ThemeHandler::execute(no-args) must return Ok(()), got: {res:?}"
        );

        let ev = rx
            .try_recv()
            .expect("ThemeHandler::execute(no-args) must emit one event");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert!(
                    text.contains("Available themes:"),
                    "no-args branch must emit a TextDelta with \
                     'Available themes:' header, got: {text}"
                );
                assert!(
                    text.contains("Usage: /theme <name>"),
                    "no-args branch must emit a TextDelta with \
                     'Usage: /theme <name>' hint, got: {text}"
                );
            }
            other => panic!(
                "no-args branch must emit TextDelta, got: {other:?}"
            ),
        }
    }

    /// A valid theme name (drawn from `available_themes()` so the test
    /// stays in lockstep with the theme registry) must emit TWO events
    /// in order: `SetTheme(name)` first (the mutation signal), then a
    /// confirmation `TextDelta`. The confirmation text must match the
    /// shipped format `"\nTheme set to '{name}'.\n"` byte-for-byte.
    #[test]
    fn theme_handler_execute_with_valid_theme_emits_settheme_and_confirmation() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ThemeHandler;
        // Pick a name guaranteed to be in `available_themes()` and also
        // a valid `theme_by_name` lookup — `intj` is the canonical
        // default theme and is the first entry in the slice.
        let name = "intj";
        // Sanity: assert the theme registry agrees so the test would
        // catch a regression in the upstream theme module rather than
        // failing mysteriously.
        assert!(
            archon_tui::theme::available_themes().contains(&name),
            "test premise broken: '{name}' must be in available_themes()"
        );
        assert!(
            archon_tui::theme::theme_by_name(name).is_some(),
            "test premise broken: theme_by_name('{name}') must return Some"
        );

        let res = h.execute(&mut ctx, &[name.to_string()]);
        assert!(
            res.is_ok(),
            "ThemeHandler::execute(valid) must return Ok(()), got: {res:?}"
        );

        // First event MUST be SetTheme(name).
        let first = rx
            .try_recv()
            .expect("valid-theme branch must emit at least one event");
        match first {
            TuiEvent::SetTheme(payload) => {
                assert_eq!(
                    payload, name,
                    "first event must be SetTheme with the resolved \
                     theme name, got: SetTheme({payload})"
                );
            }
            other => panic!(
                "valid-theme branch first event must be SetTheme, got: \
                 {other:?}"
            ),
        }

        // Second event MUST be the confirmation TextDelta with the
        // exact shipped format.
        let second = rx
            .try_recv()
            .expect("valid-theme branch must emit a confirmation event");
        match second {
            TuiEvent::TextDelta(text) => {
                let expected = format!("\nTheme set to '{name}'.\n");
                assert_eq!(
                    text, expected,
                    "valid-theme branch confirmation TextDelta must \
                     match shipped format byte-for-byte"
                );
            }
            other => panic!(
                "valid-theme branch second event must be TextDelta, \
                 got: {other:?}"
            ),
        }
    }

    /// An unknown theme name must emit a single `Error` event with the
    /// shipped format `"Unknown theme '{name}'. Available: {names}"`.
    /// Confirms that the name list is appended via `available_themes()`
    /// joined with `", "`.
    #[test]
    fn theme_handler_execute_with_unknown_theme_emits_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ThemeHandler;
        let bogus = "this-is-not-a-real-theme-xyz";
        // Defensive sanity: confirm the bogus name really is unknown so
        // a future addition to the theme registry can't silently make
        // this test pass via the wrong branch.
        assert!(
            archon_tui::theme::theme_by_name(bogus).is_none(),
            "test premise broken: '{bogus}' must NOT resolve to a theme"
        );

        let res = h.execute(&mut ctx, &[bogus.to_string()]);
        assert!(
            res.is_ok(),
            "ThemeHandler::execute(unknown) must return Ok(()), got: \
             {res:?}"
        );

        let ev = rx
            .try_recv()
            .expect("unknown-theme branch must emit one event");
        match ev {
            TuiEvent::Error(text) => {
                assert!(
                    text.starts_with(&format!("Unknown theme '{bogus}'.")),
                    "unknown-theme branch must emit an Error starting \
                     with the shipped 'Unknown theme '{{name}}'.' \
                     prefix, got: {text}"
                );
                assert!(
                    text.contains("Available: "),
                    "unknown-theme branch must list available themes \
                     after the error prefix, got: {text}"
                );
                // Spot-check that at least one known name appears in
                // the Available list — guards against a regression
                // that returns an empty join.
                assert!(
                    text.contains("intj"),
                    "unknown-theme branch's Available list must include \
                     known themes, got: {text}"
                );
            }
            other => panic!(
                "unknown-theme branch must emit Error, got: {other:?}"
            ),
        }
    }

    /// Defensive test for R4: passing a 2-token args slice (spec drift
    /// defence — current themes are all single words but the parser
    /// could in theory feed multi-token args) must not panic. The
    /// handler's join-and-trim logic should:
    /// * Either treat the joined string as an unknown theme and emit
    ///   an Error (current behaviour for any 2-word combo since no
    ///   current theme has spaces).
    /// * Or, if a future theme registry adds a multi-word name like
    ///   `"high contrast"`, treat it as a valid theme and emit
    ///   SetTheme + confirmation.
    /// Either outcome is acceptable; the only hard requirement is that
    /// `execute` returns `Ok(())` without panic and emits at least one
    /// event.
    #[test]
    fn theme_handler_execute_joins_multi_token_args_without_panicking() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ThemeHandler;
        let args = vec!["high".to_string(), "contrast".to_string()];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "ThemeHandler::execute(multi-token) must return Ok(()), \
             got: {res:?}"
        );

        // At least one event must land — either an Error (current
        // behaviour for unknown joined name) or a SetTheme/TextDelta
        // pair (future-proof for multi-word theme names).
        let mut event_count = 0;
        while rx.try_recv().is_ok() {
            event_count += 1;
        }
        assert!(
            event_count >= 1,
            "ThemeHandler::execute(multi-token) must emit at least one \
             event, got: {event_count}"
        );
    }
}
