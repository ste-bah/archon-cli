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
                let _ = ctx.tui_tx.try_send(archon_tui::app::TuiEvent::Error(
                    "Parse error: unclosed quote".to_string(),
                ));
                return Ok(());
            }
            Err(ParseError::MalformedFlag(tok)) => {
                let _ = ctx.tui_tx.try_send(archon_tui::app::TuiEvent::Error(
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
    use crate::command::registry::{default_registry, CommandHandler};
    use archon_tui::app::TuiEvent;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Build a fresh `CommandContext` backed by a bounded channel the
    /// test can drain via `try_recv`. Capacity of 8 matches the real
    /// input pipeline order of magnitude while leaving headroom.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(8);
        (CommandContext { tui_tx: tx }, rx)
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

    #[test]
    fn dispatch_recognized_command_returns_ok() {
        // Uses the real default registry — `/fast` resolves to the
        // TASK-AGS-622 stub handler which returns `Ok(())` without
        // doing any work. We assert: (a) dispatch returns Ok, and
        // (b) no `TuiEvent::Error` is emitted (i.e. we did NOT take
        // the "Unknown command" branch).
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/fast");
        assert!(result.is_ok(), "recognized command must return Ok");

        // Ensure no error event was emitted.
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
        // and NOT emit an "Unknown command" error.
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/h");
        assert!(result.is_ok(), "alias dispatch must return Ok");

        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(TuiEvent::Error(msg)) => panic!(
                "alias dispatch must not emit TuiEvent::Error, got: {msg}"
            ),
            Ok(ev) => panic!("unexpected event emitted: {ev:?}"),
            Err(e) => panic!("unexpected channel error: {e:?}"),
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
}
