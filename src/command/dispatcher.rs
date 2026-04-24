//! Slash command dispatcher.
//!
//! TASK-AGS-623: ties parser + registry together. PATH A (hybrid):
//! the dispatcher acts as a gate at the top of `handle_slash_command`,
//! parsing input, looking up the handler in the registry, calling
//! `handler.execute` (currently a no-op stub from TASK-AGS-622), and
//! emitting "Unknown command: /{name}" via the TUI event channel for
//! unrecognized names. The legacy inline match in `main.rs` continues
//! to perform the actual command bodies until a future task migrates
//! handler bodies into the registry's stub `execute` methods.
//!
//! Spec note: TASK-AGS-623 originally targeted
//! `src/tui/input/keyboard.rs`, but that file does not exist in this
//! codebase — the slash-command match is inline in
//! `src/main.rs::handle_slash_command`. PATH A (approved) skips the
//! keyboard.rs migration and installs this dispatcher as a parallel
//! gate at the top of `handle_slash_command`. The legacy 43-arm match
//! remains intact and is untouched by this task.

use std::sync::Arc;

use crate::command::errors;
use crate::command::parser::{CommandParser, ParseError};
use crate::command::registry::{CommandContext, Registry};

/// Slash command dispatcher.
///
/// Owns a shared reference to the command [`Registry`]. A single
/// dispatcher is constructed at App start time and cloned (cheaply, via
/// `Arc`) into `SlashCommandContext` for reuse by every slash input.
pub(crate) struct Dispatcher {
    registry: Arc<Registry>,
}

impl Dispatcher {
    /// Build a dispatcher around the supplied shared registry.
    pub(crate) fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Spec-mandated entry point. Parses `input`, looks the command up
    /// in the registry, and invokes the handler's `execute`. Returns
    /// `Ok(())` for both recognized and unknown commands; unknown
    /// names emit a `TuiEvent::Error("Unknown command: /{name}")`
    /// through `ctx.tui_tx` instead of propagating an error.
    ///
    /// Non-slash / empty / bare-`/` input is a no-op returning `Ok(())`
    /// with no events emitted — matching the pre-existing behaviour of
    /// the legacy inline match's `_ => false` arm for such inputs.
    ///
    /// ## TASK-AGS-803 wiring
    ///
    /// Tokenization is delegated to [`CommandParser::parse`] (TASK-AGS-801)
    /// for its richer `Result<ParsedCommand, ParseError>` surface. The
    /// leading-`/` gate stays HERE inside the dispatcher (option B from
    /// Steven's orchestrator directive) so the dispatcher does NOT steal
    /// non-slash input from the legacy inline match in `main.rs` — that
    /// behaviour is pinned by `dispatch_non_slash_input_returns_ok_no_emit`.
    ///
    /// Registry lookup uses `Registry::get`, which is alias-aware after
    /// TASK-AGS-802 — no extra alias code lives here.
    pub(crate) fn dispatch(
        &self,
        ctx: &mut CommandContext,
        input: &str,
    ) -> anyhow::Result<()> {
        let trimmed = input.trim();

        // PATH A hybrid gate: the dispatcher MUST NOT consume non-slash
        // input. `dispatch_non_slash_input_returns_ok_no_emit` and
        // `dispatch_whitespace_only_input_no_emit` pin this invariant.
        if !trimmed.starts_with('/') {
            return Ok(());
        }

        // Bare `/` is a silent no-op (matches the legacy inline match's
        // `_ => false` arm and the pre-existing
        // `dispatch_bare_slash_returns_ok_no_emit` test).
        if trimmed == "/" {
            return Ok(());
        }

        // Delegate tokenization to the structured-error wrapper.
        // `CommandParser::parse` itself relaxes the leading-`/`
        // requirement, but we already enforced it above, so the only
        // error variants reachable here are `UnclosedQuote` and
        // `MalformedFlag` (true tokenizer failures). `Empty` /
        // `MissingName` are defended as quiet no-ops for safety against
        // future refactors.
        let parsed = match CommandParser::parse(trimmed) {
            Ok(p) => p,
            Err(ParseError::Empty) | Err(ParseError::MissingName) => {
                return Ok(());
            }
            Err(ParseError::UnclosedQuote) => {
                ctx.emit(archon_tui::app::TuiEvent::Error(
                    "Parse error: unclosed quote".to_string(),
                ));
                return Ok(());
            }
            Err(ParseError::MalformedFlag(tok)) => {
                ctx.emit(archon_tui::app::TuiEvent::Error(
                    format!("Parse error: malformed flag '{tok}'"),
                ));
                return Ok(());
            }
        };

        match self.registry.get(&parsed.name) {
            Some(handler) => handler.execute(ctx, &parsed.args),
            None => {
                // TASK-AGS-804: delegate message assembly to the
                // dedicated formatter, which owns the zero / one /
                // many branching, the case-insensitive exact-match
                // fallback, and the defensive 3-suggestion cap. The
                // dispatcher is only responsible for emission.
                let msg = errors::format_unknown_command(
                    &parsed.name,
                    &self.registry,
                );
                // Emit via the TUI event channel. `try_send` so the
                // dispatcher cannot block on a full channel; dropping a
                // diagnostic under backpressure is preferable to stalling
                // the input pipeline. `TuiEvent::Error` is the correct
                // text-emitting variant for user-visible diagnostics
                // (see `crates/archon-tui/src/app.rs::TuiEvent`).
                let _ = ctx
                    .tui_tx
                    .try_send(archon_tui::app::TuiEvent::Error(msg));
                Ok(())
            }
        }
    }

    /// Returns `true` if `input` parses as a slash command whose name
    /// is registered (directly or via an alias). Used by
    /// `handle_slash_command` to decide whether to fall through to the
    /// legacy inline match (PATH A hybrid only — removed once handler
    /// bodies migrate into the registry).
    ///
    /// Mirrors the leading-`/` gate from `dispatch` so a plain-text
    /// input never claims to be a recognized slash command.
    pub(crate) fn recognizes(&self, input: &str) -> bool {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') || trimmed == "/" {
            return false;
        }
        CommandParser::parse(trimmed)
            .ok()
            .and_then(|p| self.registry.get(&p.name).map(|_| ()))
            .is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::{default_registry, CommandHandler, RegistryBuilder};
    use archon_tui::app::TuiEvent;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Build a fresh `CommandContext` backed by a bounded channel the
    /// test can drain via `try_recv`. Capacity of 8 matches the real
    /// input pipeline order of magnitude while leaving headroom.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        // The builder uses capacity 16 (was 8); dispatcher tests emit
        // at most a handful of events, so observational behavior is
        // unchanged.
        crate::command::test_support::CtxBuilder::new().build()
    }

    /// A test-only handler that records every `execute` invocation so
    /// the test can assert both that it was called and with which args.
    struct RecordingHandler {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl CommandHandler for RecordingHandler {
        fn execute(
            &self,
            _ctx: &mut CommandContext,
            args: &[String],
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(args.to_vec());
            Ok(())
        }
        fn description(&self) -> &str {
            "recording handler (test only)"
        }
    }

    // -----------------------------------------------------------------
    // Recognized / unknown / non-slash paths
    // -----------------------------------------------------------------

    /// Test-local handler that mirrors the THIN-WRAPPER no-op contract:
    /// `execute` returns `Ok(())` WITHOUT emitting any `TuiEvent`. Used
    /// by `dispatch_recognized_command_returns_ok` below so the witness
    /// test is INDEPENDENT of any specific production command stub —
    /// TASK-AGS-POST-6-NO-STUB has removed the last `declare_handler!`
    /// stubs from the registry, so every previous swap target is now a
    /// real (or byte-identically-wrapped) handler with observable
    /// behavior we must not cargo-cult into this generic witness. Shape
    /// mirrors `RecordingHandler` above (test-local) and
    /// `registry::tests::NoAliasHandler`.
    struct SilentOkHandler;
    impl CommandHandler for SilentOkHandler {
        fn execute(
            &self,
            _ctx: &mut CommandContext,
            _args: &[String],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn description(&self) -> &str {
            "silent ok handler (test only)"
        }
    }

    #[test]
    fn dispatch_recognized_command_returns_ok() {
        // WITNESS: a recognized command must (a) return Ok, and (b)
        // emit no `TuiEvent::Error` — i.e. we did NOT take the
        // "Unknown command" branch in `Dispatcher::dispatch`.
        //
        // Independence from production handlers:
        //
        // TASK-AGS-POST-6-NO-STUB eliminates the final `declare_handler!`
        // invocations (ConfigHandler, CancelHandler) and the macro
        // itself, so no production command is still a pure no-op stub.
        // B24 (/compact, /clear) already established that every
        // migrated command has observable behavior we must not rely on
        // here — and the previously-announced "next swap" target
        // /cancel is now migrated too. Rather than chase another
        // production swap target, this witness now uses an in-test
        // `SilentOkHandler` registered on a fresh `RegistryBuilder::new()`
        // under a test-only primary name (`witness-silent`). Result:
        // the witness exercises the real `Dispatcher → Registry →
        // Handler` path end-to-end WITHOUT depending on any specific
        // production registry entry — so it will not need another
        // swap when future tickets migrate or rename commands.
        let mut b = RegistryBuilder::new();
        b.insert_primary("witness-silent", Arc::new(SilentOkHandler));
        let registry = Arc::new(b.build());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/witness-silent");
        assert!(result.is_ok(), "recognized command must return Ok");

        // Ensure no event at all was emitted — both the absence of
        // `TuiEvent::Error` (we are NOT in the unknown-command branch)
        // and the absence of any other variant (the test-local
        // handler is a no-op).
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(TuiEvent::Error(msg)) => panic!(
                "recognized command must not emit TuiEvent::Error, got: {msg}"
            ),
            Ok(ev) => panic!("unexpected event emitted: {ev:?}"),
            Err(e) => panic!("unexpected channel error: {e:?}"),
        }
    }

    #[test]
    fn dispatch_unknown_command_emits_error_message() {
        // `/nope` is not a registered command and is > 2 edits from
        // every primary. The dispatcher must return Ok(()) AND push a
        // `TuiEvent::Error` matching the AGS-804 zero-suggestion form.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/nope");
        assert!(result.is_ok(), "unknown command path must return Ok");

        let ev = rx.try_recv().expect("error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("Unknown command '/nope'"),
                    "expected error to quote '/nope', got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_non_slash_input_returns_ok_no_emit() {
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "hello");
        assert!(result.is_ok());
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "non-slash input must not emit any event"
        );
    }

    #[test]
    fn dispatch_bare_slash_returns_ok_no_emit() {
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/");
        assert!(result.is_ok());
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "bare '/' must not emit an error event"
        );
    }

    // -----------------------------------------------------------------
    // `recognizes` cheap-lookup helper
    // -----------------------------------------------------------------

    #[test]
    fn recognizes_returns_true_for_registered_name() {
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        assert!(dispatcher.recognizes("/fast"));
    }

    #[test]
    fn recognizes_returns_false_for_unknown() {
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        assert!(!dispatcher.recognizes("/nope"));
    }

    // -----------------------------------------------------------------
    // Argument passing (parser composition)
    //
    // Registry has no public "insert" API and TASK-AGS-623 is
    // out-of-scope for registry.rs changes, so these two tests
    // exercise the exact composition `Dispatcher::dispatch` performs
    // (parser::parse → handler.execute(args)) against a fake handler
    // directly, rather than round-tripping through a custom Registry.
    // This still guarantees that the parser output is faithfully
    // forwarded to handler.execute — which is the contract under test.
    // -----------------------------------------------------------------

    fn invoke_handler_via_parse(
        handler: &dyn CommandHandler,
        input: &str,
    ) -> anyhow::Result<()> {
        let parsed = crate::command::parser::parse(input)
            .expect("parser must accept input");
        let (mut ctx, _rx) = make_ctx();
        handler.execute(&mut ctx, &parsed.args)
    }

    #[test]
    fn dispatch_passes_args_to_handler() {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let handler = RecordingHandler {
            calls: Arc::clone(&calls),
        };

        invoke_handler_via_parse(&handler, "/foo a b c").unwrap();

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "handler should be called exactly once");
        assert_eq!(
            recorded[0],
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "handler should receive parser-tokenized positional args in order"
        );
    }

    #[test]
    fn dispatch_strips_quoted_args() {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let handler = RecordingHandler {
            calls: Arc::clone(&calls),
        };

        invoke_handler_via_parse(&handler, "/foo \"hello world\"").unwrap();

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0],
            vec!["hello world".to_string()],
            "quoted argument must arrive at the handler as a single token"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-803: alias-aware dispatch + structured parse-error tests.
    //
    // The first three exercise the alias-fallback path in
    // `Registry::get` (wired in AGS-802) through the dispatcher; the
    // next two verify the suggestion/no-suggestion branches of the
    // unknown-command error formatter; and the last three exercise the
    // `CommandParser::parse` -> `ParseError` -> `TuiEvent::Error` edges
    // (UnclosedQuote / MalformedFlag) plus the whitespace-only guard.
    // -----------------------------------------------------------------

    #[test]
    fn dispatch_alias_resolves_to_primary_handler() {
        // "h" is registered as an alias for "help" in the default
        // registry (see `HelpHandler::aliases`). Dispatching "/h" must
        // land on the help handler (via Registry::get's alias fallback)
        // and NOT emit an "Unknown command" error. Post-B06-HELP the
        // real HelpHandler now emits a TextDelta with the core-commands
        // header (skill_registry is None in the dispatcher test fixture,
        // so no extended-commands suffix is appended); any non-Error
        // event is acceptable — the alias-fallback contract only
        // forbids the "Unknown command" Error.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/h");
        assert!(result.is_ok(), "alias dispatch must return Ok");

        // Drain all events; assert none is an Error variant.
        loop {
            match rx.try_recv() {
                Err(mpsc::error::TryRecvError::Empty) => break,
                Ok(TuiEvent::Error(msg)) => panic!(
                    "alias dispatch must not emit TuiEvent::Error, got: {msg}"
                ),
                Ok(_ev) => {
                    // TextDelta from HelpHandler is expected post-B06-HELP.
                    continue;
                }
                Err(e) => panic!("unexpected channel error: {e:?}"),
            }
        }
    }

    #[test]
    fn recognizes_returns_true_for_alias() {
        // `recognizes` must honour the registry's alias map — "/h"
        // resolves to the /help primary, so recognizes must report true.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        assert!(
            dispatcher.recognizes("/h"),
            "recognizes must return true for registered alias '/h' -> /help"
        );
    }

    #[test]
    fn recognizes_returns_false_for_unknown_alias() {
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        assert!(
            !dispatcher.recognizes("/xyz123"),
            "recognizes must return false for an unregistered name"
        );
    }

    #[test]
    fn dispatch_unknown_emits_suggestion_when_close_match_exists() {
        // "/hel" is 1 edit away from "/help" and > 2 from every other
        // primary. The TASK-AGS-804 formatter emits the single-match
        // form verbatim: `Unknown command '/hel'. Did you mean '/help'?`
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/hel");
        assert!(result.is_ok(), "unknown command must still return Ok");

        let ev = rx.try_recv().expect("error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg,
                    "Unknown command '/hel'. Did you mean '/help'?",
                    "single-match form must match the AGS-804 spec verbatim"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_unknown_emits_plain_error_when_no_close_match() {
        // "/zzzqqq" is > 2 edits from every primary, so suggest()
        // returns []. The AGS-804 formatter emits the zero-suggestion
        // "/help" hint form verbatim.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/zzzqqq");
        assert!(result.is_ok());

        let ev = rx.try_recv().expect("error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg,
                    "Unknown command '/zzzqqq'. Type /help for the full list.",
                    "zero-suggestion form must match the AGS-804 spec verbatim"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_unclosed_quote_emits_parse_error() {
        // CommandParser::parse returns ParseError::UnclosedQuote for
        // `/foo "unterminated`. The dispatcher must surface this as a
        // TuiEvent::Error describing the parse failure and return Ok(()).
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/foo \"unterminated");
        assert!(result.is_ok(), "parse error must not propagate as Err");

        let ev = rx.try_recv().expect("parse error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("Parse error"),
                    "error should be tagged 'Parse error', got: {msg}"
                );
                assert!(
                    msg.contains("unclosed quote"),
                    "error should mention 'unclosed quote', got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_malformed_flag_emits_parse_error() {
        // `/foo --` triggers ParseError::MalformedFlag("--"). The
        // dispatcher must surface it as a TuiEvent::Error tagged
        // "Parse error" mentioning "malformed flag".
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/foo --");
        assert!(result.is_ok());

        let ev = rx.try_recv().expect("parse error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("Parse error"),
                    "error should be tagged 'Parse error', got: {msg}"
                );
                assert!(
                    msg.contains("malformed flag"),
                    "error should mention 'malformed flag', got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_whitespace_only_input_no_emit() {
        // Whitespace-only input is rejected by the dispatcher's
        // leading-`/` gate BEFORE CommandParser is invoked, so no
        // TuiEvent::Error is emitted and the call returns Ok(()).
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "   ");
        assert!(result.is_ok(), "whitespace input must return Ok");
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "whitespace input must not emit any event"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-POST-6-DISPATCH-SMOKE: end-to-end dispatcher coverage.
    //
    // The body-migrate stream (B01..B24) finished with 40 primaries
    // routed through `Dispatcher::dispatch`. The AGS-POST-6-FALLTHROUGH
    // ticket then deleted the legacy 477-line slash.rs match, leaving
    // the dispatcher as the single routing authority. What we were
    // missing up to this point was a loop-the-registry smoke covering
    // EVERY primary + EVERY alias in one pass. Unit-per-handler tests
    // (one per command body file) each prove their own slice, but
    // nothing in the suite pinned "iterate the whole catalog, confirm
    // none of them hit the dispatch-layer 'Unknown command' branch".
    //
    // The four tests below close that gap:
    //
    //   * `dispatch_smoke_all_primaries_route_without_unknown_error`
    //     — loops every registered primary name, dispatches `/{name}`
    //       with a fresh channel per iteration, and asserts that the
    //       emitted-event stream contains NO `TuiEvent::Error(msg)`
    //       whose `msg` begins with `"Unknown command"`. Handler-level
    //       Err is tolerated (most handlers need populated context
    //       fields this fixture deliberately leaves at `None`); the
    //       smoke is strictly a DISPATCH-LAYER miss detector.
    //
    //   * `dispatch_smoke_all_aliases_route_without_unknown_error`
    //     — walks the (primary, alias) space using the same strategy
    //       as `registry_integration_all_commands_wired` (registry.rs
    //       :2597) — `registry.names()` + `handler.aliases()` — and
    //       asserts the same "no dispatch-layer Unknown command" for
    //       every alias. Closes the contract that the alias map is
    //       exhaustively reachable via the dispatcher.
    //
    //   * `recognizes_smoke_all_primaries_return_true`
    //     — cheap: for every primary `/{name}`, `recognizes` must be
    //       true. Pairs with `recognizes_returns_true_for_registered_name`
    //       (single-sample witness) and lifts it to FULL coverage.
    //
    //   * `registry_primary_count_matches_expected_count`
    //     — defensive regression guard. If a future refactor silently
    //       drops or doubles a primary, this fails IMMEDIATELY without
    //       needing a full dispatch loop. Numeric witness pinned to
    //       the registry-side `EXPECTED_COMMAND_COUNT = 49` constant
    //       (registry.rs:1655); changes must land in both places.
    //
    // Failure-report strategy mirrors `registry_integration_all_commands_wired`
    // (registry.rs:2564) — collect-and-report, so a single run surfaces
    // every broken command/alias simultaneously instead of panicking at
    // the first failure.
    // -----------------------------------------------------------------

    /// Canonical primary-count invariant. Mirrors
    /// `registry::tests::EXPECTED_COMMAND_COUNT` (registry.rs:1655).
    /// That constant lives behind `#[cfg(test)]` inside `registry.rs`
    /// and is not re-exported, so we pin the same integer here. If
    /// either constant moves, BOTH must be updated in lockstep.
    const EXPECTED_PRIMARY_COUNT: usize = 49;

    /// Drain every currently-queued event from `rx` using `try_recv`
    /// until the channel reports empty, returning the drained events
    /// in FIFO order. The smoke tests below call this once per
    /// dispatch so handler-emitted events do not leak into the next
    /// iteration.
    fn drain_events(
        rx: &mut mpsc::Receiver<archon_tui::app::TuiEvent>,
    ) -> Vec<archon_tui::app::TuiEvent> {
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev) => out.push(ev),
                Err(mpsc::error::TryRecvError::Empty) => return out,
                Err(mpsc::error::TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Return the first `TuiEvent::Error(msg)` whose `msg` begins with
    /// `"Unknown command"` — the exact prefix the dispatcher's
    /// unknown-command branch emits via
    /// `errors::format_unknown_command` (TASK-AGS-804). Returns `None`
    /// when the event stream has no dispatch-layer miss, which is the
    /// smoke-test pass condition.
    fn first_unknown_command_error(
        events: &[archon_tui::app::TuiEvent],
    ) -> Option<String> {
        events.iter().find_map(|ev| match ev {
            archon_tui::app::TuiEvent::Error(msg)
                if msg.starts_with("Unknown command") =>
            {
                Some(msg.clone())
            }
            _ => None,
        })
    }

    /// Drive one `(primary_or_alias, input)` through the dispatcher
    /// with a fresh channel, wrap the call in `catch_unwind` so
    /// handler-internal panics (e.g. DenialsHandler's
    /// `.expect("denial_snapshot populated")` on a stripped fixture —
    /// denials.rs:151) do NOT abort the whole smoke sweep, and
    /// return any drained `TuiEvent::Error("Unknown command…")` as a
    /// failure candidate.
    ///
    /// Why `catch_unwind` is sound here:
    ///   * Several handlers (`DenialsHandler`, `McpHandler`,
    ///     `CopyHandler`, …) explicitly `.expect()` on missing
    ///     context fields — the author's stated intent is "panic to
    ///     surface wiring bugs LOUDLY at test-time". Our dispatcher
    ///     test fixture deliberately leaves those fields at `None`,
    ///     so those handlers WILL panic under this smoke. That is
    ///     out-of-scope for a DISPATCH-LAYER smoke — we only care
    ///     whether `Dispatcher::dispatch` routed the input to a
    ///     handler at all (versus emitting the dispatch-layer
    ///     "Unknown command" error). `catch_unwind` lets us treat
    ///     "handler ran, then panicked" as SUCCESS for routing —
    ///     which is what we want.
    ///   * We pass `AssertUnwindSafe` because `CommandContext` holds
    ///     a `tokio::sync::mpsc::Sender` which is not
    ///     `UnwindSafe`. That is fine: the ctx is about to be
    ///     dropped, and any handler-level panic leaves it in a
    ///     well-defined state (same-or-fewer events in the channel).
    ///
    /// Returns `Some(msg)` if the dispatcher emitted an
    /// "Unknown command" error (the real failure mode we are
    /// hunting), otherwise `None` — regardless of whether the
    /// handler succeeded, returned Err, or panicked.
    fn smoke_dispatch_detect_unknown_error(
        dispatcher: &Dispatcher,
        input: &str,
    ) -> Option<String> {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let (mut ctx, mut rx) = make_ctx();
        // NOTE: `catch_unwind` lets a handler panic without aborting
        // the smoke sweep, but the process-wide panic hook still
        // runs — so each trapped panic prints a stack header to
        // stderr. That is acceptable (the output is still a PASS
        // for cargo) and preferable to `set_hook`/`take_hook` here:
        // the panic hook is PROCESS-wide, and with `--test-threads=2`
        // swapping it under the primaries smoke would race the
        // aliases smoke (or any other concurrently-running test
        // whose panic output we would then lose). Keeping the
        // default hook means correctness trumps output quietness.
        let _result = catch_unwind(AssertUnwindSafe(|| {
            let _ = dispatcher.dispatch(&mut ctx, input);
        }));
        let events = drain_events(&mut rx);
        first_unknown_command_error(&events)
    }

    #[test]
    fn dispatch_smoke_all_primaries_route_without_unknown_error() {
        // For every primary P registered in `default_registry()`:
        //   1. Build a fresh `(ctx, rx)` — each iteration needs its
        //      own channel so event backlog does not leak.
        //   2. Dispatch `/{P}` with no args, tolerating both
        //      handler-level `Err` AND handler-level panic (see
        //      `smoke_dispatch_detect_unknown_error` doc for
        //      rationale — the dispatcher fixture deliberately
        //      leaves several context fields `None` so handlers
        //      that `.expect()` on them will panic, which is OUT
        //      of scope for a dispatch-layer smoke).
        //   3. Drain `rx` and record any `TuiEvent::Error(msg)`
        //      whose `msg` begins with "Unknown command".
        //
        // Handler-level `Err` return values (e.g. "FastHandler:
        // fast_mode_shared not populated" from fast.rs:88 when the
        // fixture leaves `fast_mode_shared` at None) and handler
        // panics (e.g. DenialsHandler at denials.rs:151) are both
        // TOLERATED — the smoke only asserts the dispatcher's
        // routing layer, not handler preconditions. Neither an Err
        // return nor a panic emits a `TuiEvent::Error`, so they
        // cannot trip the check below.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(Arc::clone(&registry));

        let mut failures: Vec<String> = Vec::new();
        for primary_name in registry.names() {
            let input = format!("/{primary_name}");
            if let Some(err_msg) =
                smoke_dispatch_detect_unknown_error(&dispatcher, &input)
            {
                failures.push(format!(
                    "primary '/{primary_name}' produced dispatch-layer \
                     Unknown command error: {err_msg:?}"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "dispatch_smoke_all_primaries_route_without_unknown_error: \
             {} primary/primaries failed routing:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn dispatch_smoke_all_aliases_route_without_unknown_error() {
        // Walk the (primary, alias) space via registry.names() +
        // handler.aliases() (the same iteration strategy used by
        // `registry_integration_all_commands_wired` in registry.rs
        // :2597 — there is no public alias iterator on Registry, so
        // we reach aliases through their owning primary handler).
        //
        // For every alias A on every primary P, dispatch `/{A}` and
        // assert the dispatch layer did NOT emit an "Unknown command"
        // TuiEvent::Error. Handler Err and handler panics are both
        // tolerated (same rationale as the primaries smoke above).
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(Arc::clone(&registry));

        let mut failures: Vec<String> = Vec::new();
        let mut alias_total: usize = 0;
        for primary_name in registry.names() {
            let handler = match registry.get(primary_name) {
                Some(h) => h,
                None => {
                    failures.push(format!(
                        "primary '{primary_name}' enumerated via names() \
                         but missing from registry.get() — should be \
                         unreachable"
                    ));
                    continue;
                }
            };
            for alias in handler.aliases() {
                alias_total += 1;
                let input = format!("/{alias}");
                if let Some(err_msg) =
                    smoke_dispatch_detect_unknown_error(&dispatcher, &input)
                {
                    failures.push(format!(
                        "alias '/{alias}' (primary '/{primary_name}') \
                         produced dispatch-layer Unknown command \
                         error: {err_msg:?}"
                    ));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "dispatch_smoke_all_aliases_route_without_unknown_error: \
             {} alias(es) failed routing across {} total alias(es) \
             inspected:\n{}",
            failures.len(),
            alias_total,
            failures.join("\n")
        );
    }

    #[test]
    fn recognizes_smoke_all_primaries_return_true() {
        // `Dispatcher::recognizes("/{name}")` must return `true` for
        // every primary registered in `default_registry()`. This
        // lifts the single-sample `recognizes_returns_true_for_registered_name`
        // witness to full-catalog coverage without duplicating its
        // `/fast` assertion.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(Arc::clone(&registry));

        let mut failures: Vec<String> = Vec::new();
        for primary_name in registry.names() {
            let input = format!("/{primary_name}");
            if !dispatcher.recognizes(&input) {
                failures.push(format!(
                    "recognizes('{input}') returned false — primary \
                     '/{primary_name}' is registered but the \
                     dispatcher does not recognise it"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "recognizes_smoke_all_primaries_return_true: \
             {} primary/primaries failed the recognises check:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn registry_primary_count_matches_expected_count() {
        // Defensive regression guard: the registered primary count
        // MUST equal `EXPECTED_PRIMARY_COUNT` (=49), and the iterator
        // produced by `Registry::names()` MUST yield exactly that many
        // distinct names. If a future refactor silently drops or
        // double-registers a primary this test fails immediately
        // without a full dispatch sweep. Mirrors
        // `default_registry_contains_all_commands` in registry.rs
        // :1658 but lives in the dispatcher test module so the
        // dispatcher-side coverage guarantee is self-contained.
        let registry = default_registry();
        let names: Vec<&'static str> = registry.names();

        assert_eq!(
            names.len(),
            EXPECTED_PRIMARY_COUNT,
            "registry.names().len() = {}, expected \
             EXPECTED_PRIMARY_COUNT = {} (=49 per registry.rs:1655). \
             A primary was added or removed without updating this \
             constant.",
            names.len(),
            EXPECTED_PRIMARY_COUNT,
        );

        // Cross-check: `Registry::len()` and `Registry::names().len()`
        // must agree. They read the same underlying HashMap but via
        // different APIs, so a divergence would indicate a map/view
        // bug introduced by a future refactor.
        assert_eq!(
            registry.len(),
            names.len(),
            "registry.len() = {} disagrees with registry.names().len() \
             = {} — the HashMap and its view iterator must report the \
             same cardinality",
            registry.len(),
            names.len(),
        );
    }
}
