//! `/context` handler coverage, split out for the 500-line ceiling (#192).
//!
//! Declared with `#[path]` from `context_cmd.rs`, so `super` still means that
//! module and the assertions read exactly as they did in place.

use super::*;

/// Build a `CommandContext` with a freshly-created channel and the
/// supplied optional context snapshot. Tests exercising the
/// defensive None branch pass `None`; tests exercising the happy
/// path pass `Some(ContextSnapshot { .. })`.
fn make_ctx(
    snapshot: Option<ContextSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_context_snapshot_opt(snapshot)
        .build()
}
/// A snapshot with nothing measured yet — enough for the handler to run.
fn empty_snapshot() -> ContextSnapshot {
    ContextSnapshot {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        turn_count: 0,
        system_prompt_chars: 0,
        tool_defs_chars: 0,
        context_window: 1_000_000,
        context_source: "catalog".into(),
        last_request_body_tokens: 0,
    }
}

#[test]
fn context_handler_description_matches() {
    let h = ContextHandler;
    let desc = h.description().to_lowercase();
    assert!(
        desc.contains("context") || desc.contains("window") || desc.contains("usage"),
        "ContextHandler description should reference \
         context/window/usage, got: {}",
        h.description()
    );
}

#[test]
fn context_handler_aliases_are_empty() {
    let h = ContextHandler;
    assert_eq!(
        h.aliases(),
        &[] as &[&'static str],
        "ContextHandler must register NO aliases — shipped stub's \
         `ctx` alias was cosmetic (legacy match arm only matched \
         `/context` literally). See module rustdoc."
    );
}

#[test]
fn context_handler_execute_with_snapshot_emits_text_delta() {
    let snap = ContextSnapshot {
        input_tokens: 1_000,
        output_tokens: 500,
        cache_creation_tokens: 10,
        cache_read_tokens: 20,
        turn_count: 3,
        system_prompt_chars: 4_000,
        tool_defs_chars: 2_000,
        context_window: 1_000_000,
        context_source: "catalog".into(),
        last_request_body_tokens: 470_000,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    ContextHandler
        .execute(&mut ctx, &[])
        .expect("ContextHandler::execute must return Ok with snapshot");

    let ev = rx.try_recv().expect("must emit a TuiEvent");
    match ev {
        TuiEvent::TextDelta(s) => {
            // Byte-for-byte anchors from the shipped format string.
            assert!(
                s.contains("Context window usage"),
                "text must include header; got: {s}"
            );
            assert!(
                s.contains("System prompt:"),
                "text must include system prompt line; got: {s}"
            );
            assert!(
                s.contains("Tool definitions:"),
                "text must include tool defs line; got: {s}"
            );
            assert!(
                s.contains("Total context:"),
                "text must include total line; got: {s}"
            );
            assert!(
                s.contains("Source:           catalog"),
                "text must include context source; got: {s}"
            );
            assert!(
                s.contains("Cache:  create 10 / read 20 tokens"),
                "text must include cache stats; got: {s}"
            );
            assert!(
                s.contains("Turns:"),
                "text must include turn count; got: {s}"
            );
            // Turn count rendered as raw integer.
            assert!(
                s.contains("Turns:  3"),
                "turn count 3 must surface verbatim; got: {s}"
            );
            // Issue #37: fixed overhead is 4000/4 + 2000/4 = 1500 tokens,
            // rendered by fmt_tok as "1.5k".
            assert!(
                s.contains("Fixed overhead:   ~1.5k tokens (resent every request)"),
                "fixed prompt/tool overhead must be a named subtotal; got: {s}"
            );
            // Issue #37: latest request-body pressure, distinct from both
            // the window percentage and the billed input tokens.
            assert!(
                s.contains("Last request:     ~470.0k tokens"),
                "latest request-body pressure must surface; got: {s}"
            );
        }
        other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
    }
}

#[test]
fn context_handler_execute_without_snapshot_returns_err() {
    let (mut ctx, _rx) = make_ctx(None);
    let result = ContextHandler.execute(&mut ctx, &[]);
    assert!(
        result.is_err(),
        "ContextHandler::execute must return Err when \
         context_snapshot is None (defensive: builder bug \
         should surface loudly)"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("context_snapshot") || err_msg.contains("build_command_context"),
        "error must describe the missing snapshot, got: {err_msg}"
    );
}

#[test]
fn context_snapshot_round_trip_via_clone() {
    // Sanity: ContextSnapshot derives Debug + Clone and cloning
    // preserves every field. Required because the type is inserted
    // into Option<ContextSnapshot> in CommandContext and read back
    // by the handler.
    let snap = ContextSnapshot {
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        turn_count: 1,
        system_prompt_chars: 100,
        tool_defs_chars: 50,
        context_window: 500_000,
        context_source: "provider".into(),
        last_request_body_tokens: 12_345,
    };
    let cloned = snap.clone();
    assert_eq!(cloned.input_tokens, 100);
    assert_eq!(cloned.output_tokens, 50);
    assert_eq!(cloned.cache_creation_tokens, 0);
    assert_eq!(cloned.cache_read_tokens, 0);
    assert_eq!(cloned.turn_count, 1);
    assert_eq!(cloned.system_prompt_chars, 100);
    assert_eq!(cloned.tool_defs_chars, 50);
    assert_eq!(cloned.context_window, 500_000);
    assert_eq!(cloned.context_source, "provider");
    assert_eq!(cloned.last_request_body_tokens, 12_345);
    // Debug impl must not panic.
    let _ = format!("{snap:?}");
}

/// Issue #37: before the first request there is no wire measurement, and
/// the line must say so rather than render a misleading `~0 tokens`.
#[test]
fn context_handler_reports_no_request_sent_before_the_first_turn() {
    let snap = ContextSnapshot {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        turn_count: 0,
        system_prompt_chars: 4_000,
        tool_defs_chars: 2_000,
        context_window: 1_000_000,
        context_source: "catalog".into(),
        last_request_body_tokens: 0,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    ContextHandler
        .execute(&mut ctx, &[])
        .expect("ContextHandler::execute must return Ok with snapshot");

    match rx.try_recv().expect("must emit a TuiEvent") {
        TuiEvent::TextDelta(s) => {
            assert!(
                s.contains("Last request:     no request sent yet"),
                "an unmeasured session must say so, not report ~0; got: {s}"
            );
            // The fixed overhead is known at startup, so it is reported
            // even before any request has been sent.
            assert!(
                s.contains("Fixed overhead:   ~1.5k tokens"),
                "fixed overhead is known before the first turn; got: {s}"
            );
        }
        other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
    }
}
/// Additive, like every other restored surface (#192 scope B): the text
/// block is what a `-p` run keeps, and the overlay rides alongside it.
#[test]
fn bare_context_opens_the_attribution_overlay_as_well_as_printing() {
    let (mut ctx, mut rx) = make_ctx(Some(empty_snapshot()));
    ContextHandler.execute(&mut ctx, &[]).expect("execute");

    let mut saw_text = false;
    let mut saw_overlay = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            TuiEvent::TextDelta(_) => saw_text = true,
            TuiEvent::ShowTokenAttribution(_) => saw_overlay = true,
            _ => {}
        }
    }
    assert!(saw_text, "the text form must survive for print mode");
    assert!(saw_overlay, "bare /context must open the overlay");
}

/// `/context` has no argument forms today, and the rule for adding a
/// surface is that existing ones do not change — including the ones a user
/// reaches by typing something unexpected.
#[test]
fn an_argument_form_opens_no_overlay() {
    let (mut ctx, mut rx) = make_ctx(Some(empty_snapshot()));
    ContextHandler
        .execute(&mut ctx, &["anything".to_string()])
        .expect("execute");

    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, TuiEvent::ShowTokenAttribution(_)),
            "an argument form opened the overlay"
        );
    }
}

/// No session store attached is the `-p` and early-startup case: the
/// ranking still opens, with indices and costs and no prose.
#[test]
fn no_session_store_yields_an_empty_preview_list_rather_than_no_event() {
    let (mut ctx, mut rx) = make_ctx(Some(empty_snapshot()));
    ContextHandler.execute(&mut ctx, &[]).expect("execute");

    let mut previews = None;
    while let Ok(event) = rx.try_recv() {
        if let TuiEvent::ShowTokenAttribution(entries) = event {
            previews = Some(entries);
        }
    }
    assert_eq!(
        previews.expect("the overlay must still open"),
        Vec::new(),
        "with no log to read there is no text, and that is not a failure"
    );
}
