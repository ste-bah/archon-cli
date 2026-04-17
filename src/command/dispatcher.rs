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

use crate::command::parser;
use crate::command::parser::suggest;
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
    pub(crate) fn dispatch(
        &self,
        ctx: &mut CommandContext,
        input: &str,
    ) -> anyhow::Result<()> {
        let parsed = match parser::parse(input) {
            Some(p) => p,
            None => return Ok(()),
        };
        match self.registry.get(&parsed.name) {
            Some(handler) => handler.execute(ctx, &parsed.args),
            None => {
                // TASK-AGS-802: consume `parser::suggest` to enrich the
                // unknown-command diagnostic with a fuzzy-match hint
                // (≤ 3 candidates, ≤ 2 edits). AGS-804 will format this
                // more nicely; a plain comma-join is adequate today.
                let names = self.registry.names();
                let suggestions =
                    suggest(&parsed.name, names.iter().copied(), 3);
                let msg = if suggestions.is_empty() {
                    format!("Unknown command: /{}", parsed.name)
                } else {
                    format!(
                        "Unknown command: /{}. Did you mean: {}?",
                        parsed.name,
                        suggestions.join(", ")
                    )
                };
                // Emit via the TUI event channel. Use `try_send` so the
                // dispatcher cannot block on a full channel; dropping a
                // diagnostic on backpressure is acceptable and cannot
                // stall the input pipeline. `TuiEvent::Error` is the
                // correct text-emitting variant for user-visible error
                // diagnostics in this codebase (see
                // `crates/archon-tui/src/app.rs::TuiEvent`).
                let _ = ctx
                    .tui_tx
                    .try_send(archon_tui::app::TuiEvent::Error(msg));
                Ok(())
            }
        }
    }

    /// Returns `true` if `input` parses as a slash command whose name
    /// is registered. Used by `handle_slash_command` to decide whether
    /// to fall through to the legacy inline match (PATH A hybrid only
    /// — removed once handler bodies migrate into the registry).
    pub(crate) fn recognizes(&self, input: &str) -> bool {
        parser::parse(input)
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
        // `/nope` is not a registered command. The dispatcher must
        // return Ok(()) AND push a `TuiEvent::Error` containing the
        // literal string "Unknown command: /nope".
        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        let result = dispatcher.dispatch(&mut ctx, "/nope");
        assert!(result.is_ok(), "unknown command path must return Ok");

        let ev = rx.try_recv().expect("error event must be emitted");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("Unknown command: /nope"),
                    "expected error to contain 'Unknown command: /nope', got: {msg}"
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
        let parsed = parser::parse(input).expect("parser must accept input");
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
}
