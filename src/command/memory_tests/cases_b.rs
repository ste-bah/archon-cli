use super::*;

#[test]
fn memory_handler_execute_search_empty_query_emits_usage_error() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new());
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["search".to_string()]);
    assert!(res.is_ok(), "search(empty) must return Ok, got: {res:?}");

    let mut saw_usage = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev
            && text == "Usage: /memory search <query>"
        {
            saw_usage = true;
        }
    }
    assert!(
        saw_usage,
        "MemoryHandler::execute(search empty) must emit \
         TuiEvent::Error with the byte-for-byte usage hint"
    );
}

#[test]
fn memory_handler_execute_search_with_query_joins_multi_token_args() {
    let tm = TestMemory::new().with_recall(Ok(vec![make_mem(
        "fff11111-gggg",
        "hello-world-memory",
        "a matching content snippet",
    )]));
    let tm_arc = Arc::new(tm);
    let mem: Arc<dyn MemoryTrait> = tm_arc.clone();
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(
        &mut ctx,
        &[
            "search".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ],
    );
    assert!(
        res.is_ok(),
        "search(multi-token) must return Ok, got: {res:?}"
    );
    // Args-reconciliation assertion: the handler must have rebuilt
    // the shipped single-string semantics by joining tokens with ' '.
    let captured = tm_arc.captured_recall_query();
    assert_eq!(
        captured.as_deref(),
        Some("hello world"),
        "MemoryHandler::execute(search hello world) must forward \
         'hello world' as a single joined query (shipped split_once \
         semantics preserved)"
    );
    // Output assertion.
    let mut got_delta: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev {
            got_delta = Some(text);
        }
    }
    let text = got_delta.expect("search(with results) must emit a TextDelta event");
    assert!(
        text.contains("Memories matching \"hello world\" (1):"),
        "TextDelta must contain the result-count header, got: {text}"
    );
    assert!(
        text.contains("[fff11111]"),
        "TextDelta must contain short id, got: {text}"
    );
    assert!(
        text.contains("hello-world-memory"),
        "TextDelta must contain the memory title, got: {text}"
    );
}

#[test]
fn memory_handler_execute_search_empty_results_emits_no_match() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new().with_recall(Ok(Vec::new())));
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(
        &mut ctx,
        &["search".to_string(), "missing-token".to_string()],
    );
    assert!(
        res.is_ok(),
        "search(empty results) must return Ok, got: {res:?}"
    );
    let mut saw_no_match = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev
            && text == "\nNo memories matching \"missing-token\".\n"
        {
            saw_no_match = true;
        }
    }
    assert!(
        saw_no_match,
        "MemoryHandler::execute(search no-match) must emit the \
         byte-for-byte 'No memories matching' TextDelta"
    );
}

#[test]
fn memory_handler_execute_clear_emits_cleared_count() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new().with_clear(Ok(7)));
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["clear".to_string()]);
    assert!(res.is_ok(), "clear must return Ok, got: {res:?}");
    let mut saw_cleared = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev
            && text == "\nCleared 7 memories from the graph.\n"
        {
            saw_cleared = true;
        }
    }
    assert!(
        saw_cleared,
        "MemoryHandler::execute(clear) must emit the byte-for-byte \
         '\\nCleared 7 memories from the graph.\\n' TextDelta"
    );
}

#[test]
fn memory_handler_execute_unknown_subcommand_emits_error() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new());
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["nope".to_string()]);
    assert!(
        res.is_ok(),
        "unknown subcommand must return Ok, got: {res:?}"
    );
    let mut saw_unknown = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev
            && text
                == "Unknown memory subcommand: nope. Use list, \
                    search, or clear."
        {
            saw_unknown = true;
        }
    }
    assert!(
        saw_unknown,
        "MemoryHandler::execute(unknown) must emit the byte-for-byte \
         'Unknown memory subcommand: nope. Use list, search, or \
         clear.' TuiEvent::Error"
    );
}

/// The `truncate_str` helper must split safely on UTF-8 char
/// boundaries. Guards against regression of the
/// `is_char_boundary` check when the byte-slice fallthrough point
/// lands inside a multi-byte character. Preserved invariant from
/// the pre-migration module.
#[test]
fn truncate_str_respects_utf8_char_boundaries() {
    // Three-byte emoji-ish char (U+4E2D zh "middle") repeated.
    let s = "中".repeat(40); // 40 * 3 = 120 bytes > 80
    let out = truncate_str(&s, 80);
    assert!(
        out.ends_with("..."),
        "truncate_str must append '...' when exceeded, got: {out}"
    );
    // Must not panic and must produce valid UTF-8.
    assert!(out.is_char_boundary(out.len()));
}
