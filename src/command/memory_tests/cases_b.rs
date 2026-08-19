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

/// `/memory store` with nothing after it must not write an empty memory.
#[test]
fn memory_handler_execute_store_empty_text_emits_usage_error() {
    let tm = Arc::new(TestMemory::new());
    let mem: Arc<dyn MemoryTrait> = tm.clone();
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["store".to_string()]);
    assert!(res.is_ok(), "store(empty) must return Ok, got: {res:?}");
    assert!(
        tm.captured_store().is_none(),
        "store(empty) must not reach the graph"
    );

    let mut saw_usage = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev
            && text == STORE_USAGE
        {
            saw_usage = true;
        }
    }
    assert!(saw_usage, "store(empty) must emit the usage hint");
}

#[test]
fn memory_handler_execute_store_writes_a_manual_fact() {
    let tm = Arc::new(TestMemory::new());
    let mem: Arc<dyn MemoryTrait> = tm.clone();
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(
        &mut ctx,
        &[
            "store".to_string(),
            "deploys".to_string(),
            "need".to_string(),
            "the".to_string(),
            "VPN".to_string(),
        ],
    );
    assert!(res.is_ok(), "store must return Ok, got: {res:?}");

    let (content, title, mtype, importance, tags) =
        tm.captured_store().expect("store must reach the graph");
    // Multi-token args are rejoined, matching /memory search.
    assert_eq!(content, "deploys need the VPN");
    // `/memory list` prints titles, so a blank one leaves the memory
    // showing as a bare id and a date.
    assert_eq!(title, "deploys need the VPN");
    assert_eq!(mtype, MemoryType::Fact);
    assert!((importance - 0.5).abs() < f64::EPSILON);
    assert!(
        tags.contains(&"manual".to_string()),
        "a hand-written memory must be distinguishable from an extracted \
         one, got tags: {tags:?}"
    );

    let mut got_delta: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev {
            got_delta = Some(text);
        }
    }
    let text = got_delta.expect("store must confirm with a TextDelta");
    assert!(
        text.contains("[aabbccdd]"),
        "confirmation must carry the short id so it can be inspected, got: {text}"
    );
    assert!(
        text.contains("deploys need the VPN"),
        "confirmation must echo what was stored, got: {text}"
    );
}

/// Every other write path is bounded; a hand-typed one is not exempt. An
/// oversized memory is recalled and injected into the prompt for as long
/// as it exists.
#[test]
fn memory_handler_execute_store_rejects_oversized_text() {
    let tm = Arc::new(TestMemory::new());
    let mem: Arc<dyn MemoryTrait> = tm.clone();
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let limit = archon_memory::extraction::content_limit(MemoryType::Fact);
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["store".to_string(), "x".repeat(limit + 1)]);
    assert!(res.is_ok(), "store(oversized) must return Ok, got: {res:?}");
    assert!(
        tm.captured_store().is_none(),
        "store(oversized) must not reach the graph"
    );

    let mut saw_error = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev
            && text.contains(&format!("{limit}-character limit"))
        {
            saw_error = true;
        }
    }
    assert!(
        saw_error,
        "store(oversized) must say what the limit is, not just refuse"
    );
}

#[test]
fn memory_handler_execute_store_reports_a_graph_failure() {
    let tm = TestMemory::new().with_store(Err(MemoryError::Database("disk full".to_string())));
    let mem: Arc<dyn MemoryTrait> = Arc::new(tm);
    let (mut ctx, mut rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(&mut ctx, &["store".to_string(), "anything".to_string()]);
    assert!(res.is_ok(), "store(err) must return Ok, got: {res:?}");

    let mut saw_error = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev
            && text.starts_with("Failed to store memory:")
        {
            saw_error = true;
        }
    }
    assert!(
        saw_error,
        "a failed write must be reported, not silently swallowed"
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
                    store, search, files, prune, or clear."
        {
            saw_unknown = true;
        }
    }
    assert!(
        saw_unknown,
        "MemoryHandler::execute(unknown) must emit the byte-for-byte \
         'Unknown memory subcommand: nope. Use list, store, search, \
         prune, or clear.' TuiEvent::Error"
    );
}

/// Type is not decoration: `injection.rs` labels every recalled memory with
/// it, so a preference stored as a fact is injected as a claim about the
/// world. Importance decides how long it survives the garden.
#[test]
fn memory_handler_execute_store_honours_type_and_importance() {
    let tm = Arc::new(TestMemory::new());
    let mem: Arc<dyn MemoryTrait> = tm.clone();
    let (mut ctx, _rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(
        &mut ctx,
        &[
            "store".to_string(),
            "--type".to_string(),
            "preference".to_string(),
            "--importance".to_string(),
            "0.9".to_string(),
            "two-space".to_string(),
            "indent".to_string(),
        ],
    );
    assert!(res.is_ok(), "store(--type) must return Ok, got: {res:?}");

    let (content, _title, mtype, importance, _tags) =
        tm.captured_store().expect("store must reach the graph");
    assert_eq!(content, "two-space indent", "flags must not reach the text");
    assert_eq!(mtype, MemoryType::Preference);
    assert!((importance - 0.9).abs() < f64::EPSILON);
}

/// Rules are learned from corrections and injected into `<rules>` on every
/// turn; snapshots are serialised state. Neither is a note to jot down.
#[test]
fn memory_handler_execute_store_rejects_types_it_does_not_own() {
    for bad in ["rule", "personality_snapshot", "nonsense"] {
        let tm = Arc::new(TestMemory::new());
        let mem: Arc<dyn MemoryTrait> = tm.clone();
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(
            &mut ctx,
            &[
                "store".to_string(),
                "--type".to_string(),
                bad.to_string(),
                "text".to_string(),
            ],
        );
        assert!(res.is_ok(), "store(--type {bad}) must return Ok");
        assert!(
            tm.captured_store().is_none(),
            "--type {bad} must not reach the graph"
        );
        let mut saw_error = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev
                && text.contains("Unknown memory type")
            {
                saw_error = true;
            }
        }
        assert!(saw_error, "--type {bad} must say what is allowed");
    }
}

#[test]
fn memory_handler_execute_store_rejects_out_of_range_importance() {
    for bad in ["1.5", "-0.1", "high"] {
        let tm = Arc::new(TestMemory::new());
        let mem: Arc<dyn MemoryTrait> = tm.clone();
        let (mut ctx, mut rx) = make_ctx(Some(mem));
        let h = MemoryHandler;
        let res = h.execute(
            &mut ctx,
            &[
                "store".to_string(),
                "--importance".to_string(),
                bad.to_string(),
                "text".to_string(),
            ],
        );
        assert!(res.is_ok(), "store(--importance {bad}) must return Ok");
        assert!(
            tm.captured_store().is_none(),
            "--importance {bad} must not reach the graph"
        );
        let mut saw_error = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(text) = ev
                && text.contains("--importance")
            {
                saw_error = true;
            }
        }
        assert!(saw_error, "--importance {bad} must be reported");
    }
}

/// A `--type` in the middle of a sentence is part of the sentence.
#[test]
fn store_options_only_consume_leading_flags() {
    let opts = StoreOptions::parse("the flag is --type fact by default").expect("parse");
    assert_eq!(opts.text, "the flag is --type fact by default");
    assert_eq!(opts.memory_type, MemoryType::Fact);
}

#[test]
fn store_options_strip_quotes_after_the_flags() {
    let opts = StoreOptions::parse("--type decision \"we chose Cozo\"").expect("parse");
    assert_eq!(opts.text, "we chose Cozo");
    assert_eq!(opts.memory_type, MemoryType::Decision);
}

/// The cookbook's documented form is quoted. Quotes that survive into
/// the stored text are then injected into the prompt on every recall.
#[test]
fn memory_handler_execute_store_drops_surrounding_quotes() {
    let tm = Arc::new(TestMemory::new());
    let mem: Arc<dyn MemoryTrait> = tm.clone();
    let (mut ctx, _rx) = make_ctx(Some(mem));
    let h = MemoryHandler;
    let res = h.execute(
        &mut ctx,
        &[
            "store".to_string(),
            "\"deploys".to_string(),
            "need".to_string(),
            "the".to_string(),
            "VPN\"".to_string(),
        ],
    );
    assert!(res.is_ok(), "store(quoted) must return Ok, got: {res:?}");
    let (content, ..) = tm.captured_store().expect("store must reach the graph");
    assert_eq!(content, "deploys need the VPN");
}

/// An apostrophe is far more likely to be part of the sentence than a
/// quoting mistake, so an unmatched quote is left alone.
#[test]
fn unquote_leaves_unmatched_quotes_alone() {
    assert_eq!(unquote("it's fine"), "it's fine");
    assert_eq!(unquote("\"half quoted"), "\"half quoted");
    assert_eq!(unquote("\""), "\"");
    assert_eq!(unquote("'single'"), "single");
    assert_eq!(unquote("plain"), "plain");
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
