use super::*;

// ── Gate 5 dispatcher-integration tests ──────────────────────
//
// Prove end-to-end routing via the real `Dispatcher` +
// `default_registry()` harness: (a) slash input hits
// PermissionsHandler (not the deleted slash.rs arm, not some other
// handler), (b) byte-identity of the shipped TextDelta strings
// survives the full dispatch chain, (c) the HYBRID pattern's two
// slots (permissions_snapshot READ, pending_effect ASYNC-WRITE)
// wire correctly through the dispatcher.
//
// Reference template: src/command/effort.rs dispatcher tests
// (B11-EFFORT Gate 5). Zero mocks: Arc<Registry> from
// `default_registry()` + `Dispatcher::new` exactly as the live
// harness builds them in session.rs.

#[test]
fn dispatcher_routes_slash_permissions_to_handler_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);

    // Bare "/permissions" → READ branch → TextDelta with snapshot
    // mode. The handler reads `permissions_snapshot` (populated
    // here inline — in production the builder fills it before
    // dispatch). Use "default" as the harness default.
    let snap = PermissionsSnapshot {
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));

    let result = dispatcher.dispatch(&mut ctx, "/permissions");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/permissions\") must return Ok; got: {result:?}"
    );

    // 1. NO pending_effect (empty-arg branch is READ-only).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end bare `/permissions` must NOT stash a \
         CommandEffect; got: {:?}",
        ctx.pending_effect
    );

    // 2. Exactly one TextDelta whose payload is byte-identical to
    //    the shipped format!() output for the snapshot mode.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "end-to-end bare `/permissions` must emit exactly one \
         event; got: {events:?}"
    );
    let expected = format!(
        "\nCurrent permission mode: {}\n\
         Usage: /permissions <mode>\n\
         Modes: default, acceptEdits, plan, auto, dontAsk, \
         bypassPermissions\n\
         Legacy aliases: ask -> default, yolo -> \
         bypassPermissions\n",
        "default"
    );
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "end-to-end bare `/permissions` TextDelta must \
                 match shipped format! byte-for-byte"
            );
        }
        other => panic!(
            "end-to-end bare `/permissions` must emit \
             TuiEvent::TextDelta, got: {other:?}"
        ),
    }

    // 3. NO Error event.
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        !has_error,
        "end-to-end bare `/permissions` must emit NO \
         TuiEvent::Error; got: {events:?}"
    );
}

#[test]
fn dispatcher_routes_slash_permissions_with_plan_arg_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::CommandEffect;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    // WRITE branch still needs snapshot for the bypass-allow
    // guard (handler defensively re-reads snapshot on WRITE).
    // Production builder always populates when primary resolves
    // to /permissions, so match that contract here.
    let snap = PermissionsSnapshot {
        current_mode: "default".to_string(),
        allow_bypass_permissions: false,
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));

    // "/permissions plan" → WRITE branch → effect stash +
    // TextDelta confirmation.
    let result = dispatcher.dispatch(&mut ctx, "/permissions plan");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/permissions plan\") must return \
         Ok; got: {result:?}"
    );

    // 1. pending_effect MUST be Some(SetPermissionMode("plan")).
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetPermissionMode(resolved)) => {
            assert_eq!(
                resolved, "plan",
                "SetPermissionMode must carry the resolved mode \
                 string from the dispatched arg"
            );
        }
        other => panic!(
            "expected Some(CommandEffect::SetPermissionMode(\"plan\")), \
             got: {other:?}"
        ),
    }

    // 2. Exactly one TextDelta whose payload is byte-identical to
    //    the shipped success format!() output.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "end-to-end `/permissions plan` must emit exactly one \
         event; got: {events:?}"
    );
    let expected = "\nPermission mode set to plan.\n".to_string();
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "end-to-end `/permissions plan` TextDelta must \
                 match shipped format! byte-for-byte"
            );
        }
        other => panic!(
            "end-to-end `/permissions plan` must emit \
             TuiEvent::TextDelta, got: {other:?}"
        ),
    }

    // 3. NO Error event.
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        !has_error,
        "end-to-end `/permissions plan` must emit NO \
         TuiEvent::Error; got: {events:?}"
    );
}
