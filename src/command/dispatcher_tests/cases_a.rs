use super::*;

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
        Ok(TuiEvent::Error(msg)) => {
            panic!("recognized command must not emit TuiEvent::Error, got: {msg}")
        }
        Ok(ev) => panic!("unexpected event emitted: {ev:?}"),
        Err(e) => panic!("unexpected channel error: {e:?}"),
    }
}

#[test]
fn dispatch_recognized_handler_error_is_user_visible() {
    let mut b = RegistryBuilder::new();
    b.insert_primary("workflow", Arc::new(FailingHandler));
    let registry = Arc::new(b.build());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_ctx();

    let result = dispatcher.dispatch(&mut ctx, "/workflow run --live build the PRD");

    assert!(result.is_err(), "handler error must still propagate");
    let ev = rx.try_recv().expect("handler error event must be emitted");
    match ev {
        TuiEvent::Error(msg) => {
            assert!(
                msg.contains("Command /workflow failed"),
                "error should identify the failed slash command, got: {msg}"
            );
            assert!(
                msg.contains("workflow command requires working directory context"),
                "error should preserve the handler failure, got: {msg}"
            );
        }
        other => panic!("expected TuiEvent::Error, got {other:?}"),
    }
    assert!(
        matches!(rx.try_recv(), Ok(TuiEvent::SlashCommandComplete)),
        "handler error must complete the slash-command lifecycle"
    );
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
fn dispatch_preserves_cli_flag_tokens_for_handlers() {
    let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let handler = RecordingHandler {
        calls: Arc::clone(&calls),
    };

    invoke_handler_via_parse(
        &handler,
        "/video ingest https://example.test/watch?v=1 --frames hybrid --asr whisper-cpp --yes",
    )
    .unwrap();

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "handler should be called exactly once");
    assert_eq!(
        recorded[0],
        vec![
            "ingest".to_string(),
            "https://example.test/watch?v=1".to_string(),
            "--frames".to_string(),
            "hybrid".to_string(),
            "--asr".to_string(),
            "whisper-cpp".to_string(),
            "--yes".to_string()
        ],
        "CLI mirror handlers must receive the original flag tokens"
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
            Ok(TuiEvent::Error(msg)) => {
                panic!("alias dispatch must not emit TuiEvent::Error, got: {msg}")
            }
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
                msg, "Unknown command '/hel'. Did you mean '/help'?",
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
                msg, "Unknown command '/zzzqqq'. Type /help for the full list.",
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
