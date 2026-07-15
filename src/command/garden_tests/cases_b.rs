use super::*;

/// Stats Err path: drive `memory_count` (the first call inside
/// `format_garden_stats`) to return an error. The handler must
/// emit `TuiEvent::Error(format!("Garden stats failed: {e}"))`.
#[test]
fn garden_handler_execute_stats_with_err_memory_emits_error() {
    let tm = TestMemory::new_empty().with_count_error("boom-stats");
    let mem: Arc<dyn MemoryTrait> = Arc::new(tm);
    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(GardenConfig::default()));
    let h = GardenHandler;
    let res = h.execute(&mut ctx, &["stats".to_string()]);
    assert!(
        res.is_ok(),
        "stats Err branch must still return Ok (error is emitted \
         via TuiEvent::Error, not surfaced via Err), got: {res:?}"
    );

    let mut got: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev {
            got = Some(text);
        }
    }
    let text = got.expect(
        "stats Err path must emit a TuiEvent::Error with the 'Garden \
         stats failed' prefix",
    );
    // `format!("{e}")` on MemoryError::Database(msg) renders
    // "database error: {msg}" per the thiserror annotation.
    assert_eq!(
        text, "Garden stats failed: database error: boom-stats",
        "stats Err payload must equal format!(\"Garden stats \
         failed: {{e}}\") byte-for-byte (shipped legacy arm semantics)"
    );
}

// ---------------------------------------------------------------
// Consolidate branch — Ok + Err paths
// ---------------------------------------------------------------

/// Consolidate Ok path: running the full six-phase consolidation
/// over an empty memory graph produces a `GardenReport` with zero
/// counts across every phase. The `duration_ms` field is
/// non-deterministic but every OTHER field of the formatted output
/// is stable, so we assert that the emitted TextDelta starts with
/// "\n", ends with "\n", and contains the "Memory Garden —
/// Consolidation Complete" header + "Before: 0 memories" / "After:
/// 0 memories" lines produced by `GardenReport::format`.
#[test]
fn garden_handler_execute_consolidate_with_ok_memory_emits_formatted_report() {
    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
    let cfg = GardenConfig::default();
    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(cfg));
    let h = GardenHandler;
    // Default branch — empty args vec.
    let res = h.execute(&mut ctx, &[]);
    assert!(res.is_ok(), "consolidate Ok must return Ok, got: {res:?}");

    let mut got: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::TextDelta(text) = ev {
            got = Some(text);
        }
    }
    let text = got.expect(
        "consolidate Ok path must emit a TextDelta event with the \
         formatted report payload",
    );
    // Leading + trailing newline invariant from
    // format!("\n{formatted}\n").
    assert!(
        text.starts_with('\n'),
        "consolidate Ok payload must start with '\\n' (shipped \
         format!(\"\\n{{formatted}}\\n\") invariant), got: {text:?}"
    );
    assert!(
        text.ends_with('\n'),
        "consolidate Ok payload must end with '\\n' (shipped \
         format!(\"\\n{{formatted}}\\n\") invariant), got: {text:?}"
    );
    // GardenReport::format header preservation.
    assert!(
        text.contains("Memory Garden — Consolidation Complete"),
        "consolidate Ok payload must contain the GardenReport \
         header 'Memory Garden — Consolidation Complete', got: \
         {text}"
    );
    // Empty-graph counters — deterministic under empty TestMemory.
    assert!(
        text.contains("Before: 0 memories"),
        "consolidate Ok payload must contain 'Before: 0 memories' \
         when running over an empty graph, got: {text}"
    );
    assert!(
        text.contains("After:  0 memories"),
        "consolidate Ok payload must contain 'After:  0 memories' \
         when running over an empty graph, got: {text}"
    );
    assert!(
        text.contains("Duplicates merged:    0"),
        "consolidate Ok payload must contain 'Duplicates merged: \
         0' row from GardenReport::format, got: {text}"
    );
}

/// Consolidate Err path: drive `memory_count` to return an error
/// (consolidate calls `memory_count` first for `total_before`).
/// The handler must emit
/// `TuiEvent::Error(format!("Garden consolidation failed: {e}"))`.
#[test]
fn garden_handler_execute_consolidate_with_err_memory_emits_error() {
    let tm = TestMemory::new_empty().with_count_error("boom-consolidate");
    let mem: Arc<dyn MemoryTrait> = Arc::new(tm);
    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(GardenConfig::default()));
    let h = GardenHandler;
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "consolidate Err branch must still return Ok (error is \
         emitted via TuiEvent::Error, not surfaced via Err), got: \
         {res:?}"
    );

    let mut got: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::Error(text) = ev {
            got = Some(text);
        }
    }
    let text = got.expect(
        "consolidate Err path must emit a TuiEvent::Error with the \
         'Garden consolidation failed' prefix",
    );
    assert_eq!(
        text, "Garden consolidation failed: database error: boom-consolidate",
        "consolidate Err payload must equal format!(\"Garden \
         consolidation failed: {{e}}\") byte-for-byte (shipped \
         legacy arm semantics)"
    );
}

// ---------------------------------------------------------------
// B13 Gate 5 — dispatcher-integration tests (zero mocks).
//
// Prove end-to-end routing via the real `Dispatcher` +
// `default_registry()` harness: (a) slash input hits
// GardenHandler (not the deleted slash.rs arm, not some other
// handler), (b) byte-identity of the shipped TextDelta strings
// survives the full dispatch chain, (c) the DIRECT-pattern's
// two-subcmd branching (stats vs consolidate default) wires
// correctly through the dispatcher.
//
// Reference template: src/command/permissions.rs dispatcher tests
// (B12-PERMISSIONS Gate 5). Zero mocks: Arc<Registry> from
// `default_registry()` + `Dispatcher::new` exactly as the live
// harness builds them in session.rs.

#[test]
fn dispatcher_routes_slash_garden_stats_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);

    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
    // Pre-compute expected payload via the same memory double so
    // this assertion stays in lockstep with the archon-memory
    // formatter across future changes.
    let expected_inner = archon_memory::garden::format_garden_stats(mem.as_ref(), 10)
        .expect("format_garden_stats on empty TestMemory must succeed");
    let expected = format!("\n{expected_inner}\n");

    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(GardenConfig::default()));
    let result = dispatcher.dispatch(&mut ctx, "/garden stats");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/garden stats\") must return Ok; \
         got: {result:?}"
    );

    // 1. NO pending_effect (DIRECT pattern does not stash effects).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end `/garden stats` must NOT stash a CommandEffect \
         (DIRECT-pattern invariant); got: {:?}",
        ctx.pending_effect
    );

    // 2. Drain events and find the TextDelta.
    let mut got: Option<String> = None;
    let mut has_error = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            TuiEvent::TextDelta(text) => got = Some(text),
            TuiEvent::Error(_) => has_error = true,
            _ => {}
        }
    }
    let text = got.expect("end-to-end `/garden stats` must emit a TuiEvent::TextDelta");
    assert_eq!(
        text, expected,
        "end-to-end `/garden stats` TextDelta must match shipped \
         format!(\"\\n{{stats}}\\n\") byte-for-byte"
    );

    // 3. NO Error event.
    assert!(
        !has_error,
        "end-to-end `/garden stats` must emit NO TuiEvent::Error"
    );
}

#[test]
fn dispatcher_routes_slash_garden_consolidate_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);

    let mem: Arc<dyn MemoryTrait> = Arc::new(TestMemory::new_empty());
    let cfg = GardenConfig::default();
    let (mut ctx, mut rx) = make_ctx(Some(mem), Some(cfg));

    // Bare "/garden" → default branch → consolidate.
    let result = dispatcher.dispatch(&mut ctx, "/garden");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/garden\") must return Ok; got: {result:?}"
    );

    // 1. NO pending_effect (DIRECT pattern).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end bare `/garden` must NOT stash a CommandEffect; \
         got: {:?}",
        ctx.pending_effect
    );

    // 2. Drain events.
    let mut got: Option<String> = None;
    let mut has_error = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            TuiEvent::TextDelta(text) => got = Some(text),
            TuiEvent::Error(_) => has_error = true,
            _ => {}
        }
    }
    let text = got.expect(
        "end-to-end bare `/garden` must emit a TuiEvent::TextDelta \
         with the formatted consolidation report",
    );

    // 3. Byte-identity invariants (duration_ms is non-deterministic,
    //    so we pin the stable bytes — same approach as the unit
    //    test `garden_handler_execute_consolidate_with_ok_memory_
    //    emits_formatted_report`).
    assert!(
        text.starts_with('\n'),
        "end-to-end consolidate TextDelta must start with '\\n'; \
         got: {text:?}"
    );
    assert!(
        text.ends_with('\n'),
        "end-to-end consolidate TextDelta must end with '\\n'; \
         got: {text:?}"
    );
    assert!(
        text.contains("Memory Garden — Consolidation Complete"),
        "end-to-end consolidate TextDelta must contain the \
         GardenReport header; got: {text}"
    );
    assert!(
        text.contains("Before: 0 memories"),
        "end-to-end consolidate TextDelta must contain \
         'Before: 0 memories' (empty-graph invariant); got: {text}"
    );
    assert!(
        text.contains("After:  0 memories"),
        "end-to-end consolidate TextDelta must contain \
         'After:  0 memories' (empty-graph invariant); got: {text}"
    );

    // 4. NO Error event.
    assert!(
        !has_error,
        "end-to-end bare `/garden` must emit NO TuiEvent::Error"
    );
}
