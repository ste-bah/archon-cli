use super::*;

#[test]
fn registry_integration_all_commands_wired() {
    use crate::command::parser::CommandParser;

    let registry = default_registry();
    let mut failures: Vec<String> = Vec::new();

    // -------------------------------------------------------------
    // INVARIANT 1 — COUNT
    // -------------------------------------------------------------
    // `default_registry().len()` must equal the expected primary-
    // count constant. If the count drifts, the test names WHICH
    // direction it drifted in the failure message so the operator
    // can reconcile without re-running the test.
    let actual = registry.len();
    if actual != EXPECTED_COMMAND_COUNT {
        failures.push(format!(
            "COUNT invariant failed: expected {EXPECTED_COMMAND_COUNT}, got {actual}"
        ));
    }

    // -------------------------------------------------------------
    // INVARIANT 2 — ALIAS-RESOLUTION
    // -------------------------------------------------------------
    // For every alias declared by every primary handler, assert
    // that `registry.get(alias)` returns an `Arc` pointing at the
    // SAME allocation as `registry.get(primary)`.
    //
    // Iteration strategy: walk `registry.names()` (every primary),
    // fetch the primary handler, read `handler.aliases()` for its
    // static alias list, then do a registry lookup for each alias
    // and compare with `Arc::ptr_eq`. This walks the full
    // (primary, alias) space without needing a public iterator
    // over the private `aliases` HashMap.
    for primary_name in registry.names() {
        let primary_handler = match registry.get(primary_name) {
            Some(h) => h,
            None => {
                failures.push(format!(
                    "ALIAS-RESOLUTION invariant failed: primary '{primary_name}' \
                         enumerated via names() but missing from registry.get()"
                ));
                continue;
            }
        };
        for alias in primary_handler.aliases() {
            let alias_handler = match registry.get(alias) {
                Some(h) => h,
                None => {
                    failures.push(format!(
                        "ALIAS-RESOLUTION invariant failed: alias '{alias}' → handler '<missing>' \
                             does NOT match primary '{primary_name}' → handler '{primary}'",
                        primary = primary_name,
                    ));
                    continue;
                }
            };
            // R2: Arc::ptr_eq on Arc<dyn CommandHandler> returns
            // true iff both handles point to the same allocation.
            // Registry::get for a primary and its alias both
            // Arc::clone the SAME stored Arc, so ptr_eq must hold.
            if !Arc::ptr_eq(&primary_handler, &alias_handler) {
                failures.push(format!(
                    "ALIAS-RESOLUTION invariant failed: alias '{alias}' → handler '{alias_desc}' \
                         does NOT match primary '{primary_name}' → handler '{primary_desc}'",
                    alias_desc = alias_handler.description(),
                    primary_desc = primary_handler.description(),
                ));
            }
        }
    }

    // -------------------------------------------------------------
    // INVARIANT 3 — PARSER ROUND-TRIP
    // -------------------------------------------------------------
    // For every primary name N in default_registry():
    //   (a) `CommandParser::parse(&format!("/{N}"))` must succeed.
    //   (b) The resulting `ParsedCommand::name` must resolve to a
    //       handler in the registry via `Registry::get`.
    //
    // (a)+(b) together prove the parser recognizes every primary
    // and the registry can route the parsed output back to its
    // handler. One-directional per R3.
    for primary_name in registry.names() {
        let input = format!("/{primary_name}");
        match CommandParser::parse(&input) {
            Ok(parsed) => {
                if registry.get(&parsed.name).is_none() {
                    failures.push(format!(
                        "PARSER round-trip invariant failed: primary '{primary_name}' \
                             → parser result 'Ok(name={parsed_name:?})' → registry lookup 'None'",
                        parsed_name = parsed.name,
                    ));
                }
            }
            Err(e) => {
                failures.push(format!(
                    "PARSER round-trip invariant failed: primary '{primary_name}' \
                         → parser result 'Err({e:?})' → registry lookup '<skipped: parse failed>'"
                ));
            }
        }
    }

    // R4: collect-and-report — one test run surfaces every broken
    // command/alias simultaneously instead of panicking at the
    // first failure.
    assert!(
        failures.is_empty(),
        "registry_integration_all_commands_wired: {} invariant failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
