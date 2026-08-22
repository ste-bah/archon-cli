use super::*;

/// Build a `CommandContext` with a freshly-created channel and an
/// optional [`PermissionsSnapshot`]. All other optional fields
/// stay `None`. Mirrors the make_ctx fixtures in effort.rs /
/// add_dir.rs.
fn make_ctx(
    snapshot: Option<PermissionsSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_permissions_snapshot_opt(snapshot)
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
/// at registry.rs:914 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
#[test]
fn permissions_handler_description_byte_identical_to_shipped() {
    let h = PermissionsHandler;
    assert_eq!(
        h.description(),
        "Show or update tool permissions",
        "PermissionsHandler description must match the shipped \
         declare_handler! stub verbatim (shipped-wins drift-reconcile)"
    );
}

/// Shipped `declare_handler!` stub at registry.rs:914 used the
/// two-arg form — equivalent to `&[]`. AGS-817 shipped-wins rule
/// preserves zero aliases.
#[test]
fn permissions_handler_aliases_are_empty() {
    let h = PermissionsHandler;
    assert_eq!(
        h.aliases(),
        &[] as &[&'static str],
        "PermissionsHandler must have an empty alias slice per B12 R3 \
         (shipped declare_handler! stub had no aliases)"
    );
}

/// Bare `/permissions` (no args) must emit a single
/// `TuiEvent::TextDelta` whose payload is byte-identical to the
/// shipped multi-line format with the snapshot's current_mode
/// interpolated. NO `pending_effect` must be stashed — the
/// empty-arg branch is read-only.
#[test]
fn permissions_handler_execute_with_no_args_emits_snapshot_text() {
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = PermissionsHandler;
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "PermissionsHandler::execute(no-args) must return Ok(()), got: {res:?}"
    );

    assert!(
        ctx.pending_effect.is_none(),
        "empty-arg branch must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );

    let events = drain(&mut rx);
    // Two since #192: the shipped text, unchanged, and the overlay that shows
    // the rules the mode is qualified by. The overlay is additive — the count
    // moved, the text did not, and that is what the next assertion pins.
    assert_eq!(
        events.len(),
        2,
        "empty-arg branch must emit the text and the overlay; got: {events:?}"
    );
    let expected =
        "\nCurrent permission mode: default\nUsage: /permissions <mode>\nModes: default, acceptEdits, plan, auto, dontAsk, bypassPermissions\nLegacy aliases: ask -> default, yolo -> bypassPermissions\n"
            .to_string();
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
    match &events[1] {
        TuiEvent::ShowPermissions { mode, rules } => {
            assert_eq!(mode, "default");
            assert!(rules.is_empty(), "no rules were configured; got: {rules:?}");
        }
        other => panic!("empty-arg branch must open the rules overlay, got: {other:?}"),
    }
}

/// A valid non-bypass mode (`"plan"`) must:
/// * Emit a single `TuiEvent::TextDelta` whose payload matches the
///   shipped `format!("\nPermission mode set to {resolved}.\n")`
///   byte-for-byte.
/// * Stash `CommandEffect::SetPermissionMode("plan")` in
///   `pending_effect`.
/// * Emit NO `TuiEvent::Error`.
/// * NOT emit `PermissionModeChanged` (that is apply_effect's job).
#[test]
fn permissions_handler_execute_with_valid_plan_stashes_effect_and_emits_set_text() {
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = PermissionsHandler;
    let res = h.execute(&mut ctx, &["plan".to_string()]);
    assert!(
        res.is_ok(),
        "PermissionsHandler::execute(valid) must return Ok(()), got: {res:?}"
    );

    // 1. pending_effect MUST be Some(SetPermissionMode("plan")).
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetPermissionMode(s)) => {
            assert_eq!(
                s, "plan",
                "SetPermissionMode must carry the validated mode string"
            );
        }
        other => panic!(
            "expected Some(CommandEffect::SetPermissionMode(\"plan\")), \
             got: {other:?}"
        ),
    }

    // 2. Exactly one TextDelta event with byte-identical format.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "valid-arg branch must emit exactly one event (TextDelta); \
         PermissionModeChanged is emitted by apply_effect, not the \
         handler. got: {events:?}"
    );
    let expected = "\nPermission mode set to plan.\n".to_string();
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
}

/// `bypassPermissions` with `allow_bypass_permissions == false`
/// must:
/// * Emit a single `TuiEvent::Error` with the byte-identical
///   guard message.
/// * NOT stash any `CommandEffect`.
/// * NOT emit any `TuiEvent::TextDelta`.
#[test]
fn permissions_handler_execute_bypass_without_allow_emits_error_no_effect() {
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = PermissionsHandler;
    let res = h.execute(&mut ctx, &["bypassPermissions".to_string()]);
    assert!(
        res.is_ok(),
        "PermissionsHandler::execute(bypass-blocked) must return \
         Ok(()), got: {res:?}"
    );

    // 1. NO effect stashed.
    assert!(
        ctx.pending_effect.is_none(),
        "bypass-blocked branch must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );

    // 2. Exactly one Error event with byte-identical payload.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "bypass-blocked branch must emit exactly one event; got: {events:?}"
    );
    match &events[0] {
        TuiEvent::Error(msg) => {
            assert_eq!(
                msg, "bypassPermissions requires --allow-dangerously-skip-permissions flag",
                "bypass-blocked Error must be byte-identical to shipped"
            );
        }
        other => panic!("bypass-blocked branch must emit TuiEvent::Error, got: {other:?}"),
    }
    // 3. NO TextDelta.
    let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
    assert!(
        !has_delta,
        "bypass-blocked branch must emit NO TuiEvent::TextDelta; got: {events:?}"
    );
}

#[test]
fn permissions_bypass_denial_records_to_governed_learning_db() {
    let db = cozo::DbInstance::new("mem", "", "").expect("db");
    archon_learning::schema::ensure_learning_schema(&db).expect("schema");
    let db = std::sync::Arc::new(db);
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, _rx) = crate::command::test_support::CtxBuilder::new()
        .with_permissions_snapshot_opt(Some(snap))
        .with_governed_learning_db(std::sync::Arc::clone(&db))
        .build();
    let h = PermissionsHandler;

    h.execute(&mut ctx, &["bypassPermissions".to_string()])
        .expect("execute");

    let events = archon_learning::permission_runtime_events::list_permission_runtime_events(&db)
        .expect("permission events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tool_name, "PermissionMode");
    assert_eq!(events[0].decision, "mode_change_denied");
    assert_eq!(
        events[0].reason_code.as_deref(),
        Some("dangerous_bypass_guard")
    );
}

/// `bypassPermissions` with `allow_bypass_permissions == true`
/// must: bypass-allow succeed (fall through the normal valid
/// path): emit the confirmation TextDelta and stash the effect.
#[test]
fn permissions_handler_execute_bypass_with_allow_stashes_effect() {
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: true,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = PermissionsHandler;
    let res = h.execute(&mut ctx, &["bypassPermissions".to_string()]);
    assert!(
        res.is_ok(),
        "PermissionsHandler::execute(bypass-allowed) must return \
         Ok(()), got: {res:?}"
    );

    // 1. Effect MUST be stashed.
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetPermissionMode(s)) => {
            assert_eq!(
                s, "bypassPermissions",
                "SetPermissionMode must carry 'bypassPermissions' when allowed"
            );
        }
        other => panic!(
            "expected Some(SetPermissionMode(\"bypassPermissions\")), \
             got: {other:?}"
        ),
    }

    // 2. Confirmation TextDelta.
    let events = drain(&mut rx);
    assert_eq!(events.len(), 1, "expected one TextDelta; got: {events:?}");
    let expected = "\nPermission mode set to bypassPermissions.\n".to_string();
    match &events[0] {
        TuiEvent::TextDelta(text) => assert_eq!(text, &expected),
        other => panic!("expected TextDelta, got: {other:?}"),
    }
}

/// An invalid mode must:
/// * Emit a single `TuiEvent::Error` whose payload is byte-
///   identical to the string returned by
///   `archon_tools::validation::validate_permission_mode(arg)`.
/// * NOT stash any `CommandEffect`.
/// * NOT emit any `TuiEvent::TextDelta`.
#[test]
fn permissions_handler_execute_with_invalid_arg_emits_validation_error() {
    let snap = PermissionsSnapshot {
        rules: Vec::new(),
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
        active_preset: archon_core::config::CUSTOM_PRESET.to_string(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = PermissionsHandler;
    let bogus = "bogus-mode-xyz";
    let expected_msg = archon_tools::validation::validate_permission_mode(bogus)
        .expect_err("'bogus-mode-xyz' must NOT be a valid permission mode");

    let res = h.execute(&mut ctx, &[bogus.to_string()]);
    assert!(
        res.is_ok(),
        "PermissionsHandler::execute(invalid) must return Ok(()), got: {res:?}"
    );

    // 1. NO effect stashed.
    assert!(
        ctx.pending_effect.is_none(),
        "invalid-arg branch must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );

    // 2. Exactly one Error event with byte-identical payload.
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
                "invalid-arg branch Error must match validate_permission_mode \
                 output byte-for-byte (pass-through)"
            );
        }
        other => panic!("invalid-arg branch must emit TuiEvent::Error, got: {other:?}"),
    }
    // 3. NO TextDelta.
    let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
    assert!(
        !has_delta,
        "invalid-arg branch must emit NO TuiEvent::TextDelta; got: {events:?}"
    );
}

/// Missing snapshot on the empty-arg branch must surface as a
/// loud Err (defensive — mirrors ModelHandler/EffortHandler
/// stance against silent drift).
#[test]
fn permissions_handler_execute_no_args_without_snapshot_returns_err() {
    let (mut ctx, _rx) = make_ctx(None);
    let h = PermissionsHandler;
    let result = h.execute(&mut ctx, &[]);
    assert!(
        result.is_err(),
        "PermissionsHandler::execute must return Err when \
         permissions_snapshot is None on the empty-arg branch \
         (defensive: builder bug should surface loudly)"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("permissions_snapshot") || err_msg.contains("build_command_context"),
        "error must describe the missing snapshot, got: {err_msg}"
    );
}

#[path = "permissions_dispatcher_tests.rs"]
mod dispatcher_tests;
