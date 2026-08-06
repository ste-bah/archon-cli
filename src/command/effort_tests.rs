use super::*;

/// Build a `CommandContext` with a freshly-created channel and an
/// optional [`EffortSnapshot`]. Also exposes the sidecar field
/// `pending_effort_set` initialised to `None`.
///
/// /effort is a HYBRID handler — the READ branch reads
/// `effort_snapshot`; the WRITE branch stashes BOTH `pending_effect`
/// AND `pending_effort_set`. Every other optional field stays
/// `None`. Mirrors the make_ctx fixtures in color.rs / add_dir.rs.
fn make_ctx(
    snapshot: Option<EffortSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_effort_snapshot_opt(snapshot)
        .build()
}

/// Drain every event currently pending in the channel.
fn drain(rx: &mut archon_tui::event_channel::TuiEventReceiver) -> Vec<TuiEvent> {
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    events
}

/// The description must match the shipped `declare_handler!` stub
/// at registry.rs:801 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
#[test]
fn effort_handler_description_byte_identical_to_shipped() {
    let h = EffortHandler;
    assert_eq!(
        h.description(),
        "Show or set reasoning effort (low|medium|high|max)",
        "EffortHandler description must match the shipped \
         declare_handler! stub verbatim (shipped-wins drift-reconcile)"
    );
}

/// Shipped `declare_handler!` stub at registry.rs:801 carried no
/// alias slice — equivalent to `&[]`. AGS-817 shipped-wins rule
/// preserves zero aliases.
#[test]
fn effort_handler_aliases_are_empty() {
    let h = EffortHandler;
    assert_eq!(
        h.aliases(),
        &[] as &[&'static str],
        "EffortHandler must have an empty alias slice per B11 R3 \
         (shipped declare_handler! stub had no aliases)"
    );
}

/// Bare `/effort` (no args) must emit a single `TuiEvent::TextDelta`
/// whose payload is byte-identical to the shipped
/// `"\nCurrent effort level: {snapshot_level}\nUsage: /effort
/// <low|medium|high|max>\n"` format. NO `pending_effect` and NO
/// `pending_effort_set` must be stashed — the empty-arg branch is
/// read-only.
#[test]
fn effort_handler_execute_with_no_args_emits_snapshot_text() {
    let snap = EffortSnapshot {
        current_level: EffortLevel::Medium,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = EffortHandler;
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "EffortHandler::execute(no-args) must return Ok(()), got: {res:?}"
    );

    // Neither slot populated on READ branch.
    assert!(
        ctx.pending_effect.is_none(),
        "empty-arg branch must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );
    assert!(
        ctx.pending_effort_set.is_none(),
        "empty-arg branch must NOT stash a pending_effort_set; got: {:?}",
        ctx.pending_effort_set
    );

    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "empty-arg branch must emit exactly one event; got: {events:?}"
    );
    let expected = format!(
        "\nCurrent effort level: {}\nUsage: /effort <low|medium|high|max>\n",
        EffortLevel::Medium
    );
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "empty-arg branch TextDelta must match shipped format \
                 byte-for-byte"
            );
        }
        other => panic!("empty-arg branch must emit TuiEvent::TextDelta, got: {other:?}"),
    }
}

/// A valid effort level (`"high"`) must:
/// * Stash `CommandEffect::SetEffortLevelShared(EffortLevel::High)`
///   in `pending_effect`.
/// * Stash `EffortLevel::High` in `pending_effort_set` (SIDECAR).
/// * Emit a single `TuiEvent::TextDelta` whose payload matches the
///   shipped `format!("\nEffort level set to {level}.\n")`
///   byte-for-byte.
/// * Emit NO `TuiEvent::Error`.
#[test]
fn effort_handler_execute_with_valid_high_stashes_effect_and_sidecar_and_emits_set_text() {
    // snapshot not needed for WRITE path, pass None.
    let (mut ctx, mut rx) = make_ctx(None);
    let h = EffortHandler;
    let res = h.execute(&mut ctx, &["high".to_string()]);
    assert!(
        res.is_ok(),
        "EffortHandler::execute(valid) must return Ok(()), got: {res:?}"
    );

    // 1. pending_effect MUST be Some(SetEffortLevelShared(High)).
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetEffortLevelShared(level)) => {
            assert_eq!(
                *level,
                EffortLevel::High,
                "SetEffortLevelShared must carry the parsed EffortLevel"
            );
        }
        other => panic!(
            "expected Some(CommandEffect::SetEffortLevelShared(High)), \
             got: {other:?}"
        ),
    }

    // 2. pending_effort_set SIDECAR MUST be Some(High).
    assert_eq!(
        ctx.pending_effort_set,
        Some(EffortLevel::High),
        "pending_effort_set sidecar must carry the parsed EffortLevel"
    );

    // 3. Exactly one TextDelta event with byte-identical format.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "valid-arg branch must emit exactly one event; got: {events:?}"
    );
    let expected = format!("\nEffort level set to {}.\n", EffortLevel::High);
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "valid-arg branch TextDelta must match shipped \
                 format! byte-for-byte"
            );
        }
        other => panic!("valid-arg branch must emit TuiEvent::TextDelta, got: {other:?}"),
    }

    // 4. NO Error event.
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        !has_error,
        "valid-arg branch must emit NO TuiEvent::Error; got: {events:?}"
    );
}

/// An invalid effort level must:
/// * Emit a single `TuiEvent::Error` whose payload is byte-
///   identical to the string returned by
///   `archon_tools::validation::validate_effort_level(level_str)`.
/// * NOT stash any `CommandEffect` (pending_effect remains None).
/// * NOT stash the sidecar (pending_effort_set remains None).
/// * NOT emit any `TuiEvent::TextDelta`.
#[test]
fn effort_handler_execute_with_invalid_arg_emits_validation_error() {
    let (mut ctx, mut rx) = make_ctx(None);
    let h = EffortHandler;
    let bogus = "turbo";
    // Capture the validator's exact error message so we pin the
    // byte-identical pass-through. Any future change to the
    // validator would need to update this expectation in lockstep —
    // that is the intended coupling.
    let expected_msg = archon_tools::validation::validate_effort_level(bogus)
        .expect_err("'turbo' must NOT be a valid effort level");

    let res = h.execute(&mut ctx, &[bogus.to_string()]);
    assert!(
        res.is_ok(),
        "EffortHandler::execute(invalid) must return Ok(()), got: {res:?}"
    );

    // 1. NO effect stashed.
    assert!(
        ctx.pending_effect.is_none(),
        "invalid-arg branch must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );
    // 2. NO sidecar stashed.
    assert!(
        ctx.pending_effort_set.is_none(),
        "invalid-arg branch must NOT stash a pending_effort_set; got: {:?}",
        ctx.pending_effort_set
    );

    // 3. Exactly one Error event with byte-identical payload.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "invalid-arg branch must emit exactly one event; got: {events:?}"
    );
    match &events[0] {
        TuiEvent::Error(msg) => {
            assert_eq!(
                msg, &expected_msg,
                "invalid-arg branch Error must match validate_effort_level \
                 output byte-for-byte (pass-through)"
            );
        }
        other => panic!("invalid-arg branch must emit TuiEvent::Error, got: {other:?}"),
    }
    // 4. NO TextDelta.
    let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
    assert!(
        !has_delta,
        "invalid-arg branch must emit NO TuiEvent::TextDelta; got: {events:?}"
    );
}

/// Defensive test for R4: passing a multi-token args slice (e.g.
/// `["high", "extra"]`) must:
/// * Join with " " and trim to `"high extra"`.
/// * Not panic.
/// * Return `Ok(())`.
/// * Emit at least one event (almost certainly an Error because
///   `validate_effort_level("high extra")` fails; but a future
///   validator that accepted multi-word levels would satisfy this
///   test equally via the success branch).
#[test]
fn effort_handler_execute_joins_multi_token_args_without_panicking() {
    let (mut ctx, mut rx) = make_ctx(None);
    let h = EffortHandler;
    let args = vec!["high".to_string(), "extra".to_string()];
    let res = h.execute(&mut ctx, &args);
    assert!(
        res.is_ok(),
        "EffortHandler::execute(multi-token) must return Ok(()), got: {res:?}"
    );

    let events = drain(&mut rx);
    assert!(
        !events.is_empty(),
        "EffortHandler::execute(multi-token) must emit at least one \
         event; got: {events:?}"
    );
}

// -------------------------------------------------------------------
// Gate 5 dispatcher-integration tests — TASK-AGS-POST-6-BODIES-B11-EFFORT
// -------------------------------------------------------------------
//
// These tests drive the real `Dispatcher` + `default_registry()` +
// `EffortHandler` end-to-end, replacing the unit-level `h.execute(...)`
// harness with the same dispatch path the TUI input loop uses. They
// pin the fact that (a) registry routing for "/effort" lands on
// `EffortHandler`, (b) parser tokenization delivers args correctly
// for both bare and trailing-args forms, (c) byte-framing of shipped
// strings survives the full dispatch chain, and (d) the HYBRID
// pattern's three slots (effort_snapshot READ, pending_effect
// ASYNC-WRITE, pending_effort_set SIDECAR) wire correctly through
// the dispatcher.
//
// Reference template: src/command/add_dir.rs dispatcher tests
// (B10-ADDDIR Gate 5) — same structure. Zero mocks: Arc<Registry>
// from `default_registry()` + `Dispatcher::new` exactly as the live
// harness builds them in session.rs.

#[test]
fn dispatcher_routes_slash_effort_to_handler_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);

    // Bare "/effort" → READ branch → TextDelta with snapshot level.
    // The handler reads `effort_snapshot` (populated here inline —
    // in production the builder fills it before dispatch). Use
    // Medium as the harness default.
    let snap = EffortSnapshot {
        current_level: EffortLevel::Medium,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));

    let result = dispatcher.dispatch(&mut ctx, "/effort");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/effort\") must return Ok; got: {result:?}"
    );

    // 1. NO pending_effect (empty-arg branch is READ-only).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end bare `/effort` must NOT stash a CommandEffect; \
         got: {:?}",
        ctx.pending_effect
    );
    // 2. NO pending_effort_set sidecar (empty-arg branch is READ-only).
    assert!(
        ctx.pending_effort_set.is_none(),
        "end-to-end bare `/effort` must NOT stash a pending_effort_set \
         sidecar; got: {:?}",
        ctx.pending_effort_set
    );

    // 3. Exactly one TextDelta whose payload is byte-identical to
    //    the shipped format!() output for the snapshot level.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "end-to-end bare `/effort` must emit exactly one event; got: \
         {events:?}"
    );
    let expected = format!(
        "\nCurrent effort level: {}\nUsage: /effort <low|medium|high|max>\n",
        EffortLevel::Medium
    );
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "end-to-end bare `/effort` TextDelta must match shipped \
                 format! byte-for-byte"
            );
        }
        other => panic!(
            "end-to-end bare `/effort` must emit TuiEvent::TextDelta, \
             got: {other:?}"
        ),
    }

    // 4. NO Error event.
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        !has_error,
        "end-to-end bare `/effort` must emit NO TuiEvent::Error; got: \
         {events:?}"
    );
}

#[test]
fn dispatcher_routes_slash_effort_with_high_arg_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::CommandEffect;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    // snapshot not needed for WRITE branch, pass None.
    let (mut ctx, mut rx) = make_ctx(None);

    // "/effort high" → WRITE branch → effect stash + sidecar stash +
    // TextDelta confirmation.
    let result = dispatcher.dispatch(&mut ctx, "/effort high");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/effort high\") must return Ok; got: \
         {result:?}"
    );

    // 1. pending_effect MUST be Some(SetEffortLevelShared(High)).
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetEffortLevelShared(level)) => {
            assert_eq!(
                *level,
                EffortLevel::High,
                "SetEffortLevelShared must carry the parsed EffortLevel \
                 from the dispatched arg"
            );
        }
        other => panic!(
            "expected Some(CommandEffect::SetEffortLevelShared(High)), \
             got: {other:?}"
        ),
    }

    // 2. pending_effort_set SIDECAR MUST be Some(High).
    assert_eq!(
        ctx.pending_effort_set,
        Some(EffortLevel::High),
        "pending_effort_set sidecar must carry the parsed EffortLevel \
         from the dispatched arg"
    );

    // 3. Exactly one TextDelta whose payload is byte-identical to
    //    the shipped success format!() output.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "end-to-end `/effort high` must emit exactly one event; got: \
         {events:?}"
    );
    let expected = format!("\nEffort level set to {}.\n", EffortLevel::High);
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "end-to-end `/effort high` TextDelta must match shipped \
                 format! byte-for-byte"
            );
        }
        other => panic!(
            "end-to-end `/effort high` must emit TuiEvent::TextDelta, \
             got: {other:?}"
        ),
    }

    // 4. NO Error event.
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        !has_error,
        "end-to-end `/effort high` must emit NO TuiEvent::Error; got: \
         {events:?}"
    );
}
