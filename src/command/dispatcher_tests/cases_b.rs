use super::*;
#[test]
fn dispatch_smoke_all_primaries_route_without_unknown_error() {
    // For every primary P registered in `default_registry()`:
    //   1. Build a fresh `(ctx, rx)` — each iteration needs its
    //      own channel so event backlog does not leak.
    //   2. Dispatch `/{P}` with no args, tolerating both
    //      handler-level `Err` AND handler-level panic (see
    //      `smoke_dispatch_detect_unknown_error` doc for
    //      rationale — the dispatcher fixture deliberately
    //      leaves several context fields `None` so handlers
    //      that `.expect()` on them will panic, which is OUT
    //      of scope for a dispatch-layer smoke).
    //   3. Drain `rx` and record any `TuiEvent::Error(msg)`
    //      whose `msg` begins with "Unknown command".
    //
    // Handler-level `Err` return values (e.g. "FastHandler:
    // fast_mode_shared not populated" from fast.rs:88 when the
    // fixture leaves `fast_mode_shared` at None) and handler
    // panics (e.g. DenialsHandler at denials.rs:151) are both
    // TOLERATED — the smoke only asserts the dispatcher's
    // routing layer, not handler preconditions. Neither an Err
    // return nor a panic emits a `TuiEvent::Error`, so they
    // cannot trip the check below.
    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(Arc::clone(&registry));

    let mut failures: Vec<String> = Vec::new();
    for primary_name in registry.names() {
        let input = format!("/{primary_name}");
        if let Some(err_msg) = smoke_dispatch_detect_unknown_error(&dispatcher, &input) {
            failures.push(format!(
                "primary '/{primary_name}' produced dispatch-layer \
                 Unknown command error: {err_msg:?}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "dispatch_smoke_all_primaries_route_without_unknown_error: \
         {} primary/primaries failed routing:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn dispatch_smoke_all_aliases_route_without_unknown_error() {
    // Walk the (primary, alias) space via registry.names() +
    // handler.aliases() (the same iteration strategy used by
    // `registry_integration_all_commands_wired` in registry.rs
    // :2597 — there is no public alias iterator on Registry, so
    // we reach aliases through their owning primary handler).
    //
    // For every alias A on every primary P, dispatch `/{A}` and
    // assert the dispatch layer did NOT emit an "Unknown command"
    // TuiEvent::Error. Handler Err and handler panics are both
    // tolerated (same rationale as the primaries smoke above).
    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(Arc::clone(&registry));

    let mut failures: Vec<String> = Vec::new();
    let mut alias_total: usize = 0;
    for primary_name in registry.names() {
        let handler = match registry.get(primary_name) {
            Some(h) => h,
            None => {
                failures.push(format!(
                    "primary '{primary_name}' enumerated via names() \
                     but missing from registry.get() — should be \
                     unreachable"
                ));
                continue;
            }
        };
        for alias in handler.aliases() {
            alias_total += 1;
            let input = format!("/{alias}");
            if let Some(err_msg) = smoke_dispatch_detect_unknown_error(&dispatcher, &input) {
                failures.push(format!(
                    "alias '/{alias}' (primary '/{primary_name}') \
                     produced dispatch-layer Unknown command \
                     error: {err_msg:?}"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "dispatch_smoke_all_aliases_route_without_unknown_error: \
         {} alias(es) failed routing across {} total alias(es) \
         inspected:\n{}",
        failures.len(),
        alias_total,
        failures.join("\n")
    );
}

#[test]
fn recognizes_smoke_all_primaries_return_true() {
    // `Dispatcher::recognizes("/{name}")` must return `true` for
    // every primary registered in `default_registry()`. This
    // lifts the single-sample `recognizes_returns_true_for_registered_name`
    // witness to full-catalog coverage without duplicating its
    // `/fast` assertion.
    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(Arc::clone(&registry));

    let mut failures: Vec<String> = Vec::new();
    for primary_name in registry.names() {
        let input = format!("/{primary_name}");
        if !dispatcher.recognizes(&input) {
            failures.push(format!(
                "recognizes('{input}') returned false — primary \
                 '/{primary_name}' is registered but the \
                 dispatcher does not recognise it"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "recognizes_smoke_all_primaries_return_true: \
         {} primary/primaries failed the recognises check:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn registry_primary_count_matches_expected_count() {
    // Defensive regression guard: the registered primary count
    // MUST equal `EXPECTED_PRIMARY_COUNT` (lockstep with
    // `registry::tests::EXPECTED_COMMAND_COUNT`), and the iterator
    // produced by `Registry::names()` MUST yield exactly that many
    // distinct names. If a future refactor silently drops or
    // double-registers a primary this test fails immediately
    // without a full dispatch sweep. Mirrors
    // `default_registry_contains_all_commands` in registry.rs but
    // lives in the dispatcher test module so the dispatcher-side
    // coverage guarantee is self-contained.
    let registry = default_registry();
    let names: Vec<&'static str> = registry.names();

    assert_eq!(
        names.len(),
        EXPECTED_PRIMARY_COUNT,
        "registry.names().len() = {}, expected \
         EXPECTED_PRIMARY_COUNT = {}. The two parallel constants \
         (registry.rs::EXPECTED_COMMAND_COUNT and dispatcher.rs::\
         EXPECTED_PRIMARY_COUNT) must move in lockstep. A primary \
         was added or removed without updating one side.",
        names.len(),
        EXPECTED_PRIMARY_COUNT,
    );

    // Cross-check: `Registry::len()` and `Registry::names().len()`
    // must agree. They read the same underlying HashMap but via
    // different APIs, so a divergence would indicate a map/view
    // bug introduced by a future refactor.
    assert_eq!(
        registry.len(),
        names.len(),
        "registry.len() = {} disagrees with registry.names().len() \
         = {} — the HashMap and its view iterator must report the \
         same cardinality",
        registry.len(),
        names.len(),
    );
}
