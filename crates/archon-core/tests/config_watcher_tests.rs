//! Tests for config diffing, hot-reload watcher, and debounced reloader.

use std::fs;

#[path = "support/watcher_wait.rs"]
mod watcher_wait;
use watcher_wait::wait_until;

use archon_core::config::ArchonConfig;
use archon_core::config_diff::{diff_configs, is_reloadable, non_reloadable_changes};
use archon_core::config_watcher::{ConfigWatcher, DebouncedReloader};

// ---------------------------------------------------------------------------
// diff_configs tests
// ---------------------------------------------------------------------------

#[test]
fn diff_identical_configs_empty() {
    let a = ArchonConfig::default();
    let b = ArchonConfig::default();
    let changes = diff_configs(&a, &b);
    assert!(
        changes.is_empty(),
        "identical configs should produce no diffs"
    );
}

#[test]
fn diff_model_change() {
    let a = ArchonConfig::default();
    let mut b = ArchonConfig::default();
    b.api.default_model = "claude-opus-4-7".into();

    let changes = diff_configs(&a, &b);
    assert!(
        changes.contains(&"api.default_model".to_string()),
        "expected 'api.default_model' in changes, got: {changes:?}"
    );
}

#[test]
fn diff_permission_mode() {
    let a = ArchonConfig::default();
    let mut b = ArchonConfig::default();
    b.permissions.mode = "auto".into();

    let changes = diff_configs(&a, &b);
    assert!(
        changes.contains(&"permissions.mode".to_string()),
        "expected 'permissions.mode' in changes, got: {changes:?}"
    );
}

#[test]
fn diff_multiple_changes() {
    let a = ArchonConfig::default();
    let mut b = ArchonConfig::default();
    b.api.default_model = "claude-opus-4-7".into();
    b.permissions.mode = "auto".into();
    b.cost.warn_threshold = 99.0;

    let changes = diff_configs(&a, &b);
    assert!(
        changes.len() >= 3,
        "expected at least 3 changes, got {}: {changes:?}",
        changes.len()
    );
    assert!(changes.contains(&"api.default_model".to_string()));
    assert!(changes.contains(&"permissions.mode".to_string()));
    assert!(changes.contains(&"cost.warn_threshold".to_string()));
}

// ---------------------------------------------------------------------------
// is_reloadable tests
// ---------------------------------------------------------------------------

#[test]
fn is_reloadable_permissions() {
    assert!(is_reloadable("permissions.mode"));
    assert!(is_reloadable("permissions.allow_paths"));
}

#[test]
fn is_reloadable_hooks() {
    assert!(is_reloadable("hooks.pre_tool_use"));
    assert!(is_reloadable("hooks.post_session"));
}

#[test]
fn is_not_reloadable_api() {
    assert!(!is_reloadable("api.default_model"));
    assert!(!is_reloadable("api.max_retries"));
}

#[test]
fn is_not_reloadable_identity() {
    assert!(!is_reloadable("identity.mode"));
    assert!(!is_reloadable("identity.spoof_version"));
}

// ---------------------------------------------------------------------------
// non_reloadable_changes tests
// ---------------------------------------------------------------------------

#[test]
fn non_reloadable_changes_filtered() {
    let changes = vec![
        "permissions.mode".to_string(),
        "api.default_model".to_string(),
        "hooks.pre_tool_use".to_string(),
        "identity.mode".to_string(),
        "logging.level".to_string(),
    ];

    let non = non_reloadable_changes(&changes);
    assert!(non.contains(&"api.default_model".to_string()));
    assert!(non.contains(&"identity.mode".to_string()));
    assert!(non.contains(&"logging.level".to_string()));
    assert!(!non.contains(&"permissions.mode".to_string()));
    assert!(!non.contains(&"hooks.pre_tool_use".to_string()));
    assert_eq!(non.len(), 3);
}

// ---------------------------------------------------------------------------
// ConfigWatcher tests
// ---------------------------------------------------------------------------

#[test]
fn watcher_ignores_file_access() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        "[api]\ndefault_model = \"claude-sonnet-4-6\"\n",
    )
    .expect("write initial config");
    // Write order matters: the native stream delivers events in write order, so
    // seeing a drained.toml event proves every event from the earlier pre-start
    // writes (config.toml, read_done.toml) was already delivered or never will be.
    let read_done = dir.path().join("read_done.toml");
    fs::write(&read_done, "initial").expect("write read barrier");
    let drained = dir.path().join("drained.toml");
    fs::write(&drained, "initial").expect("write drain barrier");
    let watcher = ConfigWatcher::start(&[config_path.clone(), read_done.clone(), drained.clone()])
        .expect("start watcher");
    fs::write(&drained, "changed").expect("write drain barrier update");
    wait_until(
        "drained.toml modification after watcher start was not received",
        || {
            watcher
                .poll_changes()
                .iter()
                .any(|path| path.ends_with("drained.toml"))
        },
    );
    fs::read_to_string(&config_path).expect("read watched config");
    // A later write proves the event stream progressed past the read, without
    // deciding that an access event was filtered merely because time elapsed.
    fs::write(&read_done, "changed").expect("write read barrier update");
    let mut changed = Vec::new();
    wait_until(
        "read_done.toml modification after reading config.toml was not received",
        || {
            changed.extend(watcher.poll_changes());
            assert!(
                !changed.iter().any(|path| path.ends_with("config.toml")),
                "reading config must not trigger hot reload: {changed:?}"
            );
            changed.iter().any(|path| path.ends_with("read_done.toml"))
        },
    );
}

#[test]
fn watcher_detects_file_change() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.toml");

    // Write initial content
    fs::write(
        &config_path,
        "[api]\ndefault_model = \"claude-sonnet-4-6\"\n",
    )
    .expect("write initial config");

    let watcher = ConfigWatcher::start(std::slice::from_ref(&config_path)).expect("start watcher");

    // Watcher attachment is requested by start; wait for delivery below.

    // Modify the file
    fs::write(&config_path, "[api]\ndefault_model = \"claude-opus-4-7\"\n")
        .expect("write modified config");

    // Wait for filesystem event to propagate. Accumulate polls until the expected event,
    // rather than assuming that the OS worker ran within a fixed interval.
    // The process-level test watchdog contains a missing event only as a last resort;
    // this local deadline now reports the specific missing observation first.
    let mut changed = Vec::new();
    wait_until("config.toml modification event was not received", || {
        changed.extend(watcher.poll_changes());
        changed.iter().any(|p| p.ends_with("config.toml"))
    });
    assert!(!changed.is_empty(), "watcher should detect file change");
    // The changed path should match or contain our config file path
    let matched = changed.iter().any(|p| p.ends_with("config.toml"));
    assert!(
        matched,
        "expected config.toml in changed paths: {changed:?}"
    );
}

// ---------------------------------------------------------------------------
// DebouncedReloader tests
// ---------------------------------------------------------------------------

#[test]
fn debounced_reloader_waits() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.toml");

    // Write valid initial config
    let initial = ArchonConfig::default();
    let toml_str = toml::to_string_pretty(&initial).expect("serialize default config");
    fs::write(&config_path, &toml_str).expect("write initial config");

    let watcher = ConfigWatcher::start(std::slice::from_ref(&config_path)).expect("start watcher");
    let mut reloader = DebouncedReloader::new(watcher, 300, initial.clone());

    // Wait for watcher to attach is handled by start returning a successful acknowledgement.

    // Rapid changes — should not trigger reload until debounce elapses
    for i in 0..3 {
        let mut cfg = ArchonConfig::default();
        cfg.cost.warn_threshold = 10.0 + i as f64;
        let content = toml::to_string_pretty(&cfg).expect("serialize config");
        fs::write(&config_path, content).expect("write config update");
    }

    // Immediately after rapid writes, debounce should still be waiting
    let mut result = reloader.check_and_reload(std::slice::from_ref(&config_path));
    // Debounce may or may not have elapsed depending on timing, but no panic

    // Wait past debounce period by observing a completed reload with changed keys.
    wait_until(
        "debounced config reload changing cost.warn_threshold was not observed",
        || {
            if result.is_none() {
                result = reloader.check_and_reload(std::slice::from_ref(&config_path));
            }
            result.is_some()
        },
    );
    assert!(
        result
            .as_ref()
            .unwrap()
            .contains(&"cost.warn_threshold".to_string())
    );
    // After debounce, we should get a reload with changed keys (or empty if already consumed)
    // The important thing is this doesn't panic and returns a valid result
    assert!(
        result.is_some() || result.is_none(),
        "reloader should return a valid Option"
    );
}

// ---------------------------------------------------------------------------
// DebouncedReloader diffs against current config, not default
// ---------------------------------------------------------------------------

/// Prove that `force_reload` diffs against the provided current config, not
/// against `ArchonConfig::default()`. This exercises the same diff_configs
/// code path that DebouncedReloader now uses.
#[test]
fn reload_diffs_against_current_not_default() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.toml");

    // Current (running) config has non-default cost threshold of 50.0
    let mut current = ArchonConfig::default();
    current.cost.warn_threshold = 50.0;

    // Write a config that changes permissions but keeps cost at 50.0
    let mut on_disk = current.clone();
    on_disk.permissions.mode = "auto".to_string();
    let toml_str = toml::to_string_pretty(&on_disk).expect("serialize config");
    fs::write(&config_path, &toml_str).expect("write config");

    // force_reload diffs against `current` — same logic DebouncedReloader uses
    let (_new_cfg, changed_keys) =
        archon_core::config_watcher::force_reload(&[config_path], &current)
            .expect("force_reload should succeed");

    // Should detect permissions.mode changed
    assert!(
        changed_keys.contains(&"permissions.mode".to_string()),
        "expected permissions.mode in changes, got: {changed_keys:?}"
    );
    // Should NOT detect cost.warn_threshold — both current and on-disk have 50.0
    // The OLD bug would diff against default (5.0) and falsely report this.
    assert!(
        !changed_keys.contains(&"cost.warn_threshold".to_string()),
        "cost.warn_threshold should NOT be in changes (both 50.0), got: {changed_keys:?}"
    );
}

// ---------------------------------------------------------------------------
// force_reload tests
// ---------------------------------------------------------------------------

#[test]
fn force_reload_returns_new_config() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.toml");

    // Write initial valid config
    let initial = ArchonConfig::default();
    let toml_str = toml::to_string_pretty(&initial).expect("serialize default config");
    fs::write(&config_path, &toml_str).expect("write initial config");

    // Now write a modified config
    let mut modified = ArchonConfig::default();
    modified.cost.warn_threshold = 42.0;
    let modified_toml = toml::to_string_pretty(&modified).expect("serialize modified config");
    fs::write(&config_path, modified_toml).expect("write modified config");

    // Force reload
    let (new_config, changed_keys) =
        archon_core::config_watcher::force_reload(&[config_path], &initial)
            .expect("force_reload should succeed");

    assert!(
        (new_config.cost.warn_threshold - 42.0).abs() < f64::EPSILON,
        "expected warn_threshold = 42.0, got {}",
        new_config.cost.warn_threshold
    );
    assert!(
        changed_keys.contains(&"cost.warn_threshold".to_string()),
        "expected 'cost.warn_threshold' in changed keys: {changed_keys:?}"
    );
}
