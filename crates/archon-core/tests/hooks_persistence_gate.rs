/// GHOST-004 integration test: hook enable/disable persistence gate.
///
/// Verifies:
/// - load_all loads hooks from TOML with correct enabled defaults
/// - compute_hook_id produces stable, deterministic ids
/// - set_enabled(false) persists to hooks.local.toml
/// - set_enabled(true) restores the hook
/// - reload (load_all again) reflects overrides
/// - summaries expose correct enabled state per hook
/// - merge_overrides preserves non-[overrides] TOML sections
use std::fs;
use std::path::Path;

use archon_core::hooks::{HookCommandType, HookRegistry, compute_hook_id};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a minimal valid hooks TOML fixture to `project_root/.archon/hooks.toml`.
fn write_fixture_hooks_toml(project_root: &Path) {
    let archon_dir = project_root.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    let fixture = r#"
[hooks.PreToolUse]
matchers = [
  { matcher = "Bash", hooks = [
    { type = "command", command = "guard-secrets" }
  ]}
]
"#;
    fs::write(archon_dir.join("hooks.toml"), fixture).unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// (a-b) load_all from TOML fixture asserts enabled=true + stable id.
#[test]
fn load_all_from_toml_fixture_enabled_by_default() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    write_fixture_hooks_toml(project_dir.path());

    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let summaries = reg.summaries();

    assert_eq!(summaries.len(), 1, "one hook should be loaded");
    assert!(summaries[0].enabled, "hook must default to enabled=true");
    assert!(!summaries[0].id.is_empty(), "hook id must be non-empty");
    assert!(
        summaries[0].id.starts_with('h'),
        "hook id must start with 'h'"
    );

    // Verify id is deterministic
    let expected_id = compute_hook_id(
        &summaries[0].event,
        &HookCommandType::Command,
        "guard-secrets",
        Some("Bash"),
    );
    assert_eq!(summaries[0].id, expected_id);
}

/// (d) set_enabled(false) creates hooks.local.toml with [overrides].
#[test]
fn set_enabled_false_persists_to_local_toml() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    write_fixture_hooks_toml(project_dir.path());

    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let summaries = reg.summaries();
    let hook_id = summaries[0].id.clone();

    // Disable
    reg.set_enabled(&hook_id, false).unwrap();

    // hooks.local.toml must exist
    let local_path = project_dir.path().join(".archon/hooks.local.toml");
    assert!(local_path.exists(), "hooks.local.toml must be created");

    let content = fs::read_to_string(&local_path).unwrap();
    assert!(
        content.contains("[overrides]"),
        "must contain [overrides] section"
    );
    assert!(
        content.contains(&hook_id),
        "must contain the hook id: {}",
        hook_id
    );
    assert!(content.contains("false"), "must contain disabled value");

    // In-memory state must reflect the toggle immediately
    let summaries_after = reg.summaries();
    let toggled = summaries_after.iter().find(|s| s.id == hook_id).unwrap();
    assert!(
        !toggled.enabled,
        "hook must be disabled after set_enabled(false)"
    );
}

/// (e) reload via load_all reflects persisted overrides.
#[test]
fn reload_reflects_persisted_overrides() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    write_fixture_hooks_toml(project_dir.path());

    // First load: enabled
    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let hook_id = reg.summaries()[0].id.clone();
    assert!(reg.summaries()[0].enabled);

    // Disable and persist
    reg.set_enabled(&hook_id, false).unwrap();

    // Fresh load_all must see disabled state
    let reg2 = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let summaries2 = reg2.summaries();
    assert_eq!(summaries2.len(), 1);
    assert!(
        !summaries2[0].enabled,
        "hook must remain disabled after reload"
    );
    assert_eq!(
        summaries2[0].id, hook_id,
        "id must be stable across reloads"
    );
}

/// Toggle back to enabled after disabling.
#[test]
fn set_enabled_true_restores_hook() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    write_fixture_hooks_toml(project_dir.path());

    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let hook_id = reg.summaries()[0].id.clone();

    // Disable then re-enable
    reg.set_enabled(&hook_id, false).unwrap();
    reg.set_enabled(&hook_id, true).unwrap();

    // In-memory must be enabled
    let s = reg.summaries();
    assert!(s[0].enabled, "hook must be re-enabled");

    // Fresh load must also be enabled
    let reg2 = HookRegistry::load_all(project_dir.path(), home_dir.path());
    assert!(
        reg2.summaries()[0].enabled,
        "hook must be enabled after reload"
    );
}

/// set_enabled on unknown id writes override (harmless — no hook matches it).
#[test]
fn set_enabled_unknown_id_writes_override() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    write_fixture_hooks_toml(project_dir.path());

    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    // Writing an override for an unknown ID is OK — it just won't match any hook.
    let result = reg.set_enabled("h00000000", false);
    assert!(
        result.is_ok(),
        "unknown id override should succeed (harmless)"
    );

    // Verify it was persisted.
    let local_path = project_dir.path().join(".archon/hooks.local.toml");
    assert!(local_path.exists());
    let content = fs::read_to_string(&local_path).unwrap();
    assert!(content.contains("h00000000"));
}

/// Non-override TOML sections are preserved when writing hooks.local.toml.
#[test]
fn merge_overrides_preserves_other_sections() {
    let project_dir = TempDir::new().unwrap();
    let home_dir = TempDir::new().unwrap();
    let archon_dir = project_dir.path().join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();

    // Write hooks.toml fixture
    write_fixture_hooks_toml(project_dir.path());

    // Pre-populate hooks.local.toml with a custom section
    let existing_local = r#"
[custom]
key = "value"
"#;
    fs::write(archon_dir.join("hooks.local.toml"), existing_local).unwrap();

    let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
    let hook_id = reg.summaries()[0].id.clone();
    reg.set_enabled(&hook_id, false).unwrap();

    let content = fs::read_to_string(archon_dir.join("hooks.local.toml")).unwrap();
    assert!(content.contains("[custom]"), "custom section must survive");
    assert!(
        content.contains("key = \"value\""),
        "custom key must survive"
    );
    assert!(
        content.contains("[overrides]"),
        "overrides section must be added"
    );
}
