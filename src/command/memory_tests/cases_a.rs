use super::*;

#[test]
fn memory_handler_description_matches() {
    let h = MemoryHandler;
    assert_eq!(
        h.description(),
        "Inspect or manage long-term memory",
        "MemoryHandler description must match the shipped \
         declare_handler! stub verbatim (shipped-wins drift-reconcile)"
    );
}

#[test]
fn memory_handler_aliases_preserve_mem() {
    let h = MemoryHandler;
    assert_eq!(
        h.aliases(),
        &["mem"],
        "MemoryHandler aliases must preserve 'mem' from the shipped \
         declare_handler! stub (shipped-wins drift-reconcile — \
         dropping it would regress operators using /mem today)"
    );
}

/// When `CommandContext::memory` is `None`, execute() must return
/// Err describing the missing field. The real builder populates
/// the field unconditionally; this branch guards against test-
/// fixture or wiring regressions. Mirrors AGS-815 fork Err path.
#[test]
fn memory_handler_execute_without_memory_returns_err() {
    let (mut ctx, _rx) = make_ctx(None);
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_err(),
        "MemoryHandler::execute must return Err when memory is None \
         (builder contract violation), got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.contains("dispatched without memory"),
        "Err message must mention 'dispatched without memory' so \
         the operator can trace the wiring bug, got: {msg}"
    );
}

#[test]
fn memory_handler_execute_list_empty_emits_no_memories_stored() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new().with_list_recent(Ok(Vec::new())));
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &[]);
    assert!(res.is_ok(), "list(empty) must return Ok, got: {res:?}");

    let mut saw_empty = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev
            && text == "\nNo memories stored.\n"
        {
            saw_empty = true;
        }
    }
    assert!(
        saw_empty,
        "MemoryHandler::execute(list empty) must emit the byte-for-\
         byte TextDelta '\\nNo memories stored.\\n'"
    );
}

#[test]
fn memory_handler_execute_list_with_results_emits_recent_memories() {
    let m1 = make_mem("abcd1234-aaaa", "first title", "first content");
    let m2 = make_mem("efgh5678-bbbb", "second title", "second content");
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new().with_list_recent(Ok(vec![m1, m2])));
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["list".to_string()]);
    assert!(
        res.is_ok(),
        "list(with results) must return Ok, got: {res:?}"
    );

    let mut got_delta: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev {
            got_delta = Some(text);
        }
    }
    let text = got_delta.expect(
        "MemoryHandler::execute(list with results) must emit a \
         TextDelta event",
    );
    assert!(
        text.contains("Recent memories (2):"),
        "TextDelta must contain 'Recent memories (2):' header, got: \
         {text}"
    );
    assert!(
        text.contains("[abcd1234]"),
        "TextDelta must contain short id of first memory \
         '[abcd1234]', got: {text}"
    );
    assert!(
        text.contains("[efgh5678]"),
        "TextDelta must contain short id of second memory \
         '[efgh5678]', got: {text}"
    );
    assert!(
        text.contains("first title"),
        "TextDelta must include first memory title, got: {text}"
    );
    assert!(
        text.contains("second title"),
        "TextDelta must include second memory title, got: {text}"
    );
}
