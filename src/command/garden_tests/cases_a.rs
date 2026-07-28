use super::*;

#[test]
fn garden_handler_description_byte_identical_to_shipped() {
    let h = GardenHandler;
    assert_eq!(
        h.description(),
        "Run memory garden consolidation or show stats",
        "GardenHandler description must match the shipped \
         declare_handler! stub at registry.rs:958 byte-for-byte \
         (shipped-wins drift-reconcile)"
    );
}

#[test]
fn garden_handler_aliases_are_empty() {
    let h = GardenHandler;
    assert_eq!(
        h.aliases(),
        &[] as &[&'static str],
        "GardenHandler aliases must be empty to match the shipped \
         declare_handler! stub (two-arg form, no aliases slice)"
    );
}

// ---------------------------------------------------------------
// R3: missing-memory Err branch
// ---------------------------------------------------------------

/// When `CommandContext::memory` is `None`, execute() must return
/// Err describing the missing field. Production builder populates
/// the field unconditionally; this branch guards against test-
/// fixture or wiring regressions. Mirrors AGS-817 /memory Err path.
#[test]
fn garden_handler_execute_without_memory_handle_returns_err() {
    let (mut ctx, _rx) = make_ctx(None, Some(GardenConfig::default()));
    let h = GardenHandler;
    let res = h.execute(&mut ctx, &["stats".to_string()]);
    assert!(
        res.is_err(),
        "GardenHandler::execute must return Err when memory is None \
         (builder contract violation), got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("memory"),
        "Err message must mention 'memory' for operator traceability, \
         got: {msg}"
    );
    assert!(
        msg.contains("wiring") || msg.contains("builder"),
        "Err message must mention 'wiring' or 'builder' to locate \
         the fix site, got: {msg}"
    );
}

// ---------------------------------------------------------------
// Stats branch — Ok + Err paths
// ---------------------------------------------------------------

/// Stats Ok path: `format_garden_stats(memory, 10)` on an empty
/// memory store produces a deterministic string header. The
/// handler wraps it in `format!("\n{stats}\n")` before emission.
/// We assert the TextDelta bytes start with `"\n"` and contain the
/// shipped header line so the wrapping invariant is pinned.
#[test]
fn garden_handler_execute_stats_with_ok_memory_emits_formatted_stats() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
    // Pre-compute expected payload by calling format_garden_stats
    // directly on the same memory double. This guarantees the
    // assertion stays in lockstep with the archon-memory formatter
    // across future changes without hard-coding its exact output.
    let expected_inner = archon_memory::garden::format_garden_stats(mem.as_ref(), 10)
        .expect("format_garden_stats on empty TestMemory must succeed");
    let expected = format!("\n{expected_inner}\n");

    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(GardenConfig::default()));
    let h = GardenHandler;
    let res = h.execute(&mut ctx, &["stats".to_string()]);
    assert!(res.is_ok(), "stats Ok must return Ok, got: {res:?}");

    let mut got: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev {
            got = Some(text);
        }
    }
    let text = got.expect(
        "stats Ok path must emit a TextDelta event with the formatted \
         stats payload",
    );
    assert_eq!(
        text, expected,
        "stats Ok payload must equal format!(\"\\n{{stats}}\\n\") \
         byte-for-byte (shipped legacy arm semantics)"
    );
}
