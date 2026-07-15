use super::*;

/// R6: when `memory` is None but args are non-empty, execute
/// returns Err whose message mentions both `memory` and
/// `build_command_context` so the operator can trace the wiring
/// bug. Mirrors the AGS-817
/// `memory_handler_execute_without_memory_returns_err` precedent.
#[test]
fn execute_without_memory_returns_err() {
    let (mut ctx, _rx) = make_recall_ctx(None);
    let h = RecallHandler::new();
    let res = h.execute(&mut ctx, &["myquery".to_string()]);
    assert!(
        res.is_err(),
        "RecallHandler::execute with None memory and non-empty \
         args must return Err (builder contract violation), \
         got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.contains("memory"),
        "Err message must mention 'memory' so the operator can \
         trace the wiring bug, got: {msg}"
    );
    assert!(
        msg.contains("build_command_context"),
        "Err message must mention 'build_command_context' to pin \
         the owning builder, got: {msg}"
    );
}

/// Success path (match branch): stub returns one Memory and the
/// handler must emit the byte-identical formatted-list TextDelta
/// covering the complex per-entry format loop. Also asserts the
/// query was forwarded verbatim to `recall_memories` (args
/// reconciliation).
#[test]
fn execute_with_memory_and_matches_emits_formatted_list() {
    let stub = Arc::new(StubMemory::new(Ok(vec![make_mem(
        "abcdef1234",
        "Test Title",
        "hello world",
    )])));
    let memory: Arc<dyn MemoryTrait> = stub.clone();
    let (mut ctx, mut rx) = make_recall_ctx(Some(memory));
    let h = RecallHandler::new();
    let res = h.execute(&mut ctx, &["foo".to_string()]);
    assert!(res.is_ok(), "success path must return Ok(()), got: {res:?}");

    // Args-reconciliation assertion: the handler must have
    // forwarded `"foo"` verbatim to `recall_memories`.
    assert_eq!(
        stub.captured_query().as_deref(),
        Some("foo"),
        "RecallHandler::execute(foo) must forward 'foo' verbatim \
         to recall_memories"
    );

    let ev = rx
        .try_recv()
        .expect("formatted-list TextDelta must be emitted");
    match ev {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text,
                "\n1 memories matching 'foo':\n\n  [abcdef12] \
                 Test Title\n    hello world...\n\n",
                "TextDelta must be byte-identical to the shipped \
                 slash.rs:590-604 format (count + word 'memories' \
                 + single-quoted query + colon + blank line + \
                 two-space bracket + one-space title + newline + \
                 four-space snippet + literal '...' + blank line)"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta with formatted list, \
             got: {other:?}"
        ),
    }
}

/// Dispatcher-integration (empty-arg short-circuit). Narrow
/// `RegistryBuilder::new()` wires ONLY `/recall` with
/// `RecallHandler::new()`, then
/// `Dispatcher::dispatch(&mut ctx, "/recall")` routes through
/// the real alias+primary pipeline. Memory is None — the
/// empty-arg branch short-circuits before touching memory so no
/// stub is needed. Asserts the dispatcher's end-to-end wiring
/// (parser → registry → handler.execute) delivers the byte-
/// identical em-dash usage error.
#[test]
fn dispatcher_routes_slash_recall_with_empty_arg_emits_usage_error() {
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("recall", Arc::new(RecallHandler::new()));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    let (mut ctx, mut rx) = make_recall_ctx(None);
    let res = dispatcher.dispatch(&mut ctx, "/recall");
    assert!(
        res.is_ok(),
        "dispatcher.dispatch must return Ok(()) for the empty-arg \
         short-circuit path, got: {res:?}"
    );

    let ev = rx.try_recv().expect("usage error must be emitted");
    match ev {
        TuiEvent::Error(msg) => {
            assert_eq!(
                msg, "Usage: /recall <query> — search memories by keyword",
                "dispatcher must deliver the byte-identical \
                 em-dash usage error through the full parser → \
                 registry → handler pipeline"
            );
        }
        other => panic!(
            "expected TuiEvent::Error with em-dash usage literal, \
             got: {other:?}"
        ),
    }
}

/// Dispatcher-integration (error-surfacing path). Narrow
/// `RegistryBuilder::new()` wires ONLY `/recall`, dispatches
/// `"/recall somequery"` with `memory: None`, and asserts that
/// `Dispatcher::dispatch` surfaces the handler's Err
/// (dispatcher.rs:110 forwards `handler.execute(..)` verbatim —
/// it does NOT swallow handler-origin Errs). Mirrors the AGS-B17
/// `dispatcher_routes_slash_rename_without_session_id_returns_err`
/// precedent.
#[test]
fn dispatcher_routes_slash_recall_without_memory_returns_err() {
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("recall", Arc::new(RecallHandler::new()));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    let (mut ctx, _rx) = make_recall_ctx(None);
    let res = dispatcher.dispatch(&mut ctx, "/recall somequery");
    assert!(
        res.is_err(),
        "dispatcher.dispatch must surface handler Err when \
         memory is None (dispatcher forwards the Err verbatim), \
         got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.contains("memory") && msg.contains("build_command_context"),
        "Err message must mention both 'memory' and \
         'build_command_context', got: {msg}"
    );
}
