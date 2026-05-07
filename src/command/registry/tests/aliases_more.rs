use super::*;

#[test]
fn registry_resolves_config_aliases_settings_and_prefs() {
    let reg = default_registry();
    assert_eq!(reg.primary_for_alias("settings"), Some("config"));
    assert_eq!(reg.primary_for_alias("prefs"), Some("config"));
    assert_eq!(reg.primary_for_alias("config"), None); // primary, not alias
    assert!(!reg.is_primary("settings")); // alias-only, not a primary
    assert!(!reg.is_primary("prefs"));
    assert!(reg.is_primary("config")); // primary remains
}

#[test]
fn registry_hooks_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("hooks")
        .expect("hooks primary must be registered post AGS-812");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("hook"),
        "HooksHandler description should reference 'hook', got: {}",
        primary.description()
    );
    // `hooks` is a primary — not an alias of anything.
    assert!(reg.is_primary("hooks"));
    assert_eq!(reg.primary_for_alias("hooks"), None);
    // No alias entry points to `hooks`.
    assert!(!reg.aliases_map_contains("hooks"));
}

// -----------------------------------------------------------------
// TASK-AGS-816: /voice primary registration (no aliases). The
// /voice gap-fix adds a brand-new primary — there was NO prior
// /voice entry in the shipped match block or registry. SECOND
// Batch-3 NEW primary (after AGS-812 /hooks). Pin the invariant so
// future ticketing cannot silently introduce an alias without
// updating the registry collision-detection tests, and cannot
// silently promote a sibling handler to share the `voice` primary
// name.
// -----------------------------------------------------------------

#[test]
fn registry_voice_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("voice")
        .expect("voice primary must be registered post AGS-816");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("voice"),
        "VoiceHandler description should reference 'voice', got: {}",
        primary.description()
    );
    // `voice` is a primary — not an alias of anything.
    assert!(reg.is_primary("voice"));
    assert_eq!(reg.primary_for_alias("voice"), None);
    // No alias entry points to `voice`.
    assert!(!reg.aliases_map_contains("voice"));
}

// -----------------------------------------------------------------
// TASK-AGS-819: /theme primary registration (no aliases). The
// /theme body-migrate moves ThemeHandler out of the
// declare_handler! stub at registry.rs:607 and into
// `crate::command::theme`. Shipped stub had no alias slice; spec
// lists none; handler ships `&[]` per AGS-817 shipped-wins rule
// (zero aliases shipped → zero aliases preserved). FIFTH Batch-3
// ticket — EXPECTED_COMMAND_COUNT stays at 40 (body-migrate, not
// gap-fix). Pin the invariant so future ticketing cannot silently
// add an alias without updating the registry collision-detection
// tests.
// -----------------------------------------------------------------

#[test]
fn registry_theme_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("theme")
        .expect("theme primary must be registered post AGS-819");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("theme") || desc.contains("ui"),
        "ThemeHandler description should reference theme/ui, got: {}",
        primary.description()
    );
    // `theme` is a primary — not an alias of anything.
    assert!(reg.is_primary("theme"));
    assert_eq!(reg.primary_for_alias("theme"), None);
    // Spot-check alias-less invariant: `aliases_for` analogue —
    // no alias entry points to `theme`.
    assert!(!reg.aliases_map_contains("theme"));
}

// -----------------------------------------------------------------
// TASK-AGS-814: /context primary registration (no aliases). The
// /context body-migrate moves ContextHandler out of the
// declare_handler! stub and into `crate::command::context_cmd`.
// Shipped stub had `&["ctx"]` but the legacy match arm in slash.rs
// only matched `/context` literally — the alias was cosmetic. Real
// handler drops it to `&[]` to align with user-visible behaviour.
// Pin the invariant so future ticketing cannot silently re-add
// `ctx` (or any other alias) without updating the registry
// collision-detection tests.
// -----------------------------------------------------------------

#[test]
fn registry_context_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("context")
        .expect("context primary must be registered post AGS-814");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("context") || desc.contains("window") || desc.contains("usage"),
        "ContextHandler description should reference \
             context/window/usage, got: {}",
        primary.description()
    );
    // `context` is a primary — not an alias of anything.
    assert!(reg.is_primary("context"));
    assert_eq!(reg.primary_for_alias("context"), None);
    // No alias entry points to `context`. Also spot-check that
    // the shipped stub's `ctx` alias is GONE — AGS-814 drops it.
    assert!(!reg.aliases_map_contains("context"));
    assert!(
        !reg.aliases_map_contains("ctx"),
        "'ctx' alias must NOT be registered post AGS-814 — the \
             shipped stub had it but the legacy match arm only matched \
             `/context` literally so the alias was cosmetic"
    );
    assert_eq!(reg.primary_for_alias("ctx"), None);
}

#[test]
fn registry_mcp_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("mcp")
        .expect("mcp primary must be registered post AGS-811");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("mcp") || desc.contains("server"),
        "McpHandler description should reference mcp/server, got: {}",
        primary.description()
    );
    // `mcp` is a primary — not an alias of anything.
    assert!(reg.is_primary("mcp"));
    assert_eq!(reg.primary_for_alias("mcp"), None);
    // No alias entry points to `mcp`. Walk the aliases_map via the
    // test-only helper for a spot-check of common collision
    // candidates — none should resolve to /mcp.
    assert!(!reg.aliases_map_contains("mcp"));
}

// -----------------------------------------------------------------
// TASK-AGS-815: /fork primary registration (no aliases). The
// /fork body-migrate moves ForkHandler out of the
// declare_handler! stub at registry.rs:524 and into
// `crate::command::fork`. Shipped stub had `&[]` (no aliases);
// spec lists none; handler ships `&[]`. Pin the invariant so
// future ticketing cannot silently add an alias without updating
// the registry collision-detection tests.
// -----------------------------------------------------------------

#[test]
fn registry_fork_primary_with_no_aliases() {
    let reg = default_registry();
    let primary = reg
        .get("fork")
        .expect("fork primary must be registered post AGS-815");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("fork") || desc.contains("session"),
        "ForkHandler description should reference fork/session, \
             got: {}",
        primary.description()
    );
    // `fork` is a primary — not an alias of anything.
    assert!(reg.is_primary("fork"));
    assert_eq!(reg.primary_for_alias("fork"), None);
    // No alias entry points to `fork`.
    assert!(!reg.aliases_map_contains("fork"));
}

// -----------------------------------------------------------------
// TASK-AGS-817: /memory primary registration (alias: `mem`). The
// /memory body-migrate moves MemoryHandler out of the
// declare_handler! stub at registry.rs:521-525 and into
// `crate::command::memory`. Shipped stub carried `&["mem"]`; the
// spec (orchestrator directive) called for `&[]` but the body-
// migrate preserves `["mem"]` per shipped-wins drift-reconcile
// (dropping the alias would regress operators using /mem today).
// Pin the invariant so future ticketing cannot silently drop the
// alias or promote a sibling handler to share the `memory` primary
// name.
// -----------------------------------------------------------------

#[test]
fn registry_memory_primary_with_mem_alias() {
    let reg = default_registry();
    let primary = reg
        .get("memory")
        .expect("memory primary must be registered post AGS-817");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("memor"),
        "MemoryHandler description should reference 'memory', got: {}",
        primary.description()
    );
    // `memory` is a primary — not an alias of anything.
    assert!(reg.is_primary("memory"));
    assert_eq!(reg.primary_for_alias("memory"), None);
    // `mem` is the PRESERVED alias (shipped-wins drift-reconcile).
    assert_eq!(reg.primary_for_alias("mem"), Some("memory"));
    assert!(!reg.is_primary("mem"));
    // The alias resolves to the same handler.
    let via_alias = reg
        .get("mem")
        .expect("'mem' alias must resolve to /memory per AGS-817");
    assert_eq!(
        primary.description(),
        via_alias.description(),
        "'mem' must resolve to the same handler as /memory"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-818: /export primary registration (alias: `save`). The
// /export body-migrate (Option D / CANARY pattern, registry-hygiene
// only) moves ExportHandler out of the declare_handler! stub at
// registry.rs:513-517 and into `crate::command::export`. Shipped
// stub carried `&["save"]`; the real handler preserves the alias
// per shipped-wins drift-reconcile (AGS-817 /memory precedent).
// The real /export BODY stays in session.rs:2409-2480 — session.rs
// zero-diff invariant held since AGS-805 is preserved by Option D,
// with real body-migrate deferred to POST-STAGE-6 (ticket
// AGS-POST-6-EXPORT). Pin the invariant so future ticketing cannot
// silently drop the `save` alias or promote a sibling handler to
// share the `export` primary name.
// -----------------------------------------------------------------

#[test]
fn registry_export_primary_with_save_alias() {
    let reg = default_registry();
    let primary = reg
        .get("export")
        .expect("export primary must be registered post AGS-818");
    let desc = primary.description().to_lowercase();
    assert!(
        desc.contains("export") || desc.contains("session"),
        "ExportHandler description should reference export/session, \
             got: {}",
        primary.description()
    );
    // `export` is a primary — not an alias of anything.
    assert!(reg.is_primary("export"));
    assert_eq!(reg.primary_for_alias("export"), None);
    // `save` is the PRESERVED alias (shipped-wins drift-reconcile).
    assert_eq!(reg.primary_for_alias("save"), Some("export"));
    assert!(!reg.is_primary("save"));
    // The alias resolves to the same handler.
    let via_alias = reg
        .get("save")
        .expect("'save' alias must resolve to /export per AGS-818");
    assert_eq!(
        primary.description(),
        via_alias.description(),
        "'save' must resolve to the same handler as /export"
    );
}

#[test]
fn command_effect_debug_and_clone() {
    // Sanity: CommandEffect derives Debug + Clone and the
    // SetModelOverride variant round-trips its payload without
    // panic. Prevents accidental removal of the derives that
    // ModelHandler tests depend on for assertions.
    let e = CommandEffect::SetModelOverride("claude-sonnet-4-6".to_string());
    let cloned = e.clone();
    match cloned {
        CommandEffect::SetModelOverride(s) => {
            assert_eq!(s, "claude-sonnet-4-6");
        }
        // TASK-AGS-POST-6-BODIES-B04-DIFF: RunGitDiffStat is the
        // second variant, added by the /diff migration. This test
        // only constructs SetModelOverride, so RunGitDiffStat is
        // unreachable here; the arm exists solely to satisfy
        // exhaustiveness and guard against silent drift if a future
        // variant is added without updating this pin.
        CommandEffect::RunGitDiffStat(_) => {
            unreachable!("this test only constructs SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B10-ADDDIR: AddExtraDir is the third
        // variant, added by the /add-dir migration. This test only
        // constructs SetModelOverride, so AddExtraDir is unreachable
        // here; the arm exists solely to satisfy exhaustiveness and
        // guard against silent drift if a future variant is added
        // without updating this pin.
        CommandEffect::AddExtraDir(_) => {
            unreachable!("this test only constructs SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: SetEffortLevelShared is
        // the fourth variant, added by the /effort migration. This
        // test only constructs SetModelOverride, so
        // SetEffortLevelShared is unreachable here; the arm exists
        // solely to satisfy exhaustiveness and guard against silent
        // drift if a future variant is added without updating this
        // pin.
        CommandEffect::SetEffortLevelShared(_) => {
            unreachable!("this test only constructs SetModelOverride")
        }
        // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: SetPermissionMode is
        // the fifth variant, added by the /permissions migration. This
        // test only constructs SetModelOverride, so SetPermissionMode
        // is unreachable here; the arm exists solely to satisfy
        // exhaustiveness and guard against silent drift if a future
        // variant is added without updating this pin.
        CommandEffect::SetPermissionMode(_) => {
            unreachable!("this test only constructs SetModelOverride")
        }
    }
    // Debug impl must not panic — format! exercises it.
    let _ = format!("{e:?}");
}
