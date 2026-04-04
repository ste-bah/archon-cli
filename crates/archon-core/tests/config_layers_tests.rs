//! TDD tests for config layering (TASK-CLI-230).
//!
//! These tests import from modules that do not yet exist (`config_layers`,
//! `config_source`).  They are expected to fail to compile until the
//! production code is implemented.

use std::fs;
use std::path::PathBuf;

use toml::Value;

use archon_core::config_layers::{deep_merge_toml, discover_config_paths, load_layered_config, ConfigLayer};
use archon_core::config_source::{ConfigSourceMap, format_sources};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a unique temp directory and return its path.
/// The caller is responsible for cleaning up via `cleanup_temp_dir`.
fn make_temp_dir(label: &str) -> PathBuf {
    let id = uuid::Uuid::new_v4();
    let dir = std::env::temp_dir().join(format!("archon-test-{}-{}", label, id));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

/// Remove a temp directory tree.  Ignores errors so tests don't panic on
/// cleanup even if the test itself already failed.
fn cleanup_temp_dir(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// Convenience: parse a TOML string into `toml::Value::Table`.
fn parse_table(s: &str) -> Value {
    s.parse::<Value>().expect("invalid TOML in test fixture")
}

// ===========================================================================
// 1. deep_merge_toml
// ===========================================================================

#[test]
fn merge_scalar_overlay_wins() {
    let base = parse_table(r#"key = "a""#);
    let overlay = parse_table(r#"key = "b""#);
    let result = deep_merge_toml(base, overlay);
    assert_eq!(result.get("key").and_then(Value::as_str), Some("b"));
}

#[test]
fn merge_scalar_base_preserved_when_no_overlay() {
    let base = parse_table(r#"key = "a""#);
    let overlay = parse_table("");
    let result = deep_merge_toml(base, overlay);
    assert_eq!(result.get("key").and_then(Value::as_str), Some("a"));
}

#[test]
fn merge_array_overlay_replaces() {
    let base = parse_table("arr = [1, 2, 3]");
    let overlay = parse_table("arr = [4, 5]");
    let result = deep_merge_toml(base, overlay);
    let arr = result.get("arr").and_then(Value::as_array).expect("arr should exist");
    let ints: Vec<i64> = arr.iter().filter_map(Value::as_integer).collect();
    assert_eq!(ints, vec![4, 5]);
}

#[test]
fn merge_table_recursive() {
    let base = parse_table(
        r#"
[api]
model = "sonnet"
retries = 3
"#,
    );
    let overlay = parse_table(
        r#"
[api]
model = "opus"
"#,
    );
    let result = deep_merge_toml(base, overlay);
    let api = result.get("api").and_then(Value::as_table).expect("api table");
    assert_eq!(api.get("model").and_then(Value::as_str), Some("opus"));
    assert_eq!(api.get("retries").and_then(Value::as_integer), Some(3));
}

#[test]
fn merge_nested_tables() {
    let base = parse_table(
        r#"
[a.b]
x = 1
y = 2
"#,
    );
    let overlay = parse_table(
        r#"
[a.b]
y = 3
z = 4
"#,
    );
    let result = deep_merge_toml(base, overlay);
    let ab = result
        .get("a")
        .and_then(Value::as_table)
        .and_then(|a: &toml::map::Map<String, Value>| a.get("b"))
        .and_then(Value::as_table)
        .expect("a.b table");
    assert_eq!(ab.get("x").and_then(Value::as_integer), Some(1));
    assert_eq!(ab.get("y").and_then(Value::as_integer), Some(3));
    assert_eq!(ab.get("z").and_then(Value::as_integer), Some(4));
}

#[test]
fn merge_overlay_adds_new_keys() {
    let base = parse_table("a = 1");
    let overlay = parse_table("b = 2");
    let result = deep_merge_toml(base, overlay);
    assert_eq!(result.get("a").and_then(Value::as_integer), Some(1));
    assert_eq!(result.get("b").and_then(Value::as_integer), Some(2));
}

#[test]
fn merge_overlay_adds_new_section() {
    let base = parse_table(
        r#"
[api]
model = "sonnet"
"#,
    );
    let overlay = parse_table(
        r#"
[permissions]
mode = "auto"
"#,
    );
    let result = deep_merge_toml(base, overlay);
    assert!(result.get("api").and_then(Value::as_table).is_some());
    assert!(result.get("permissions").and_then(Value::as_table).is_some());
    let perm = result.get("permissions").and_then(Value::as_table).unwrap();
    assert_eq!(perm.get("mode").and_then(Value::as_str), Some("auto"));
}

#[test]
fn merge_empty_base() {
    let base = parse_table("");
    let overlay = parse_table(r#"key = "hello""#);
    let result = deep_merge_toml(base, overlay);
    assert_eq!(result.get("key").and_then(Value::as_str), Some("hello"));
}

#[test]
fn merge_empty_overlay() {
    let base = parse_table(r#"key = "world""#);
    let overlay = parse_table("");
    let result = deep_merge_toml(base, overlay);
    assert_eq!(result.get("key").and_then(Value::as_str), Some("world"));
}

// ===========================================================================
// 2. discover_config_paths
// ===========================================================================

#[test]
fn discover_finds_user_config() {
    let tmp = make_temp_dir("disc-user");
    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"opus\"\n").unwrap();

    let layers = discover_config_paths(Some(&user_cfg), &tmp, None);
    assert!(
        layers.iter().any(|l| l.layer == ConfigLayer::User),
        "should find user layer, got: {layers:?}"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn discover_finds_project_config() {
    let tmp = make_temp_dir("disc-proj");
    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    let proj_cfg = archon_dir.join("config.toml");
    fs::write(&proj_cfg, "[api]\ndefault_model = \"opus\"\n").unwrap();

    let layers = discover_config_paths(None, &work, None);
    assert!(
        layers.iter().any(|l| l.layer == ConfigLayer::Project),
        "should find project layer, got: {layers:?}"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn discover_finds_local_config() {
    let tmp = make_temp_dir("disc-local");
    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    let local_cfg = archon_dir.join("config.local.toml");
    fs::write(&local_cfg, "[api]\ndefault_model = \"haiku\"\n").unwrap();

    let layers = discover_config_paths(None, &work, None);
    assert!(
        layers.iter().any(|l| l.layer == ConfigLayer::Local),
        "should find local layer, got: {layers:?}"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn discover_skips_missing_layers() {
    let tmp = make_temp_dir("disc-skip");
    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"opus\"\n").unwrap();

    // work dir has no .archon/ at all
    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let layers = discover_config_paths(Some(&user_cfg), &work, None);
    assert_eq!(layers.len(), 1, "only user layer should exist");
    assert_eq!(layers[0].layer, ConfigLayer::User);
    cleanup_temp_dir(&tmp);
}

#[test]
fn discover_all_three_layers() {
    let tmp = make_temp_dir("disc-all");

    // user config
    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"sonnet\"\n").unwrap();

    // project config
    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(archon_dir.join("config.toml"), "[api]\ndefault_model = \"opus\"\n").unwrap();

    // local config
    fs::write(archon_dir.join("config.local.toml"), "[api]\ndefault_model = \"haiku\"\n").unwrap();

    let layers = discover_config_paths(Some(&user_cfg), &work, None);
    assert_eq!(layers.len(), 3, "expected 3 layers, got: {layers:?}");

    // Verify priority ordering: User < Project < Local
    assert_eq!(layers[0].layer, ConfigLayer::User);
    assert_eq!(layers[1].layer, ConfigLayer::Project);
    assert_eq!(layers[2].layer, ConfigLayer::Local);

    cleanup_temp_dir(&tmp);
}

// ===========================================================================
// 3. load_layered_config
// ===========================================================================

#[test]
fn load_user_only() {
    let tmp = make_temp_dir("load-user");
    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-opus-4-6"
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_project_overrides_user() {
    let tmp = make_temp_dir("load-proj-override");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_local_overrides_project() {
    let tmp = make_temp_dir("load-local-override");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();
    fs::write(
        archon_dir.join("config.local.toml"),
        "[api]\ndefault_model = \"claude-haiku-3-6\"\n",
    )
    .unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-haiku-3-6");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_project_inherits_user_keys() {
    let tmp = make_temp_dir("load-inherit");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-sonnet-4-6"
max_retries = 5
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    assert_eq!(config.api.max_retries, 5, "max_retries should inherit from user layer");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_project_array_replaces() {
    let tmp = make_temp_dir("load-arr-replace");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[permissions]
allow_paths = ["/a", "/b"]
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        r#"
[permissions]
allow_paths = ["/c"]
"#,
    )
    .unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("load should succeed");
    assert_eq!(
        config.permissions.allow_paths,
        vec!["/c".to_string()],
        "array should be replaced, not appended"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_settings_overlay() {
    let tmp = make_temp_dir("load-settings");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let settings_file = tmp.join("override.toml");
    fs::write(&settings_file, "[api]\ndefault_model = \"claude-opus-4-6\"\n").unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, Some(&settings_file), None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_missing_layers_silently_skipped() {
    let tmp = make_temp_dir("load-missing");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-opus-4-6\"\n").unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();
    // No .archon/ directory at all

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("missing layers should be silently skipped");
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_invalid_layer_warns_and_skips() {
    let tmp = make_temp_dir("load-invalid");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-opus-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    // Write intentionally broken TOML
    fs::write(archon_dir.join("config.toml"), "this is [[[not valid toml!!!").unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("invalid layer should be skipped, not crash");
    // User config should still be used
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    cleanup_temp_dir(&tmp);
}

// ===========================================================================
// 4. config_source tracking
// ===========================================================================

#[test]
fn source_tracks_user_origin() {
    let tmp = make_temp_dir("src-user");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-opus-4-6"
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::User),
        "api.default_model should be attributed to user layer"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn source_tracks_override() {
    let tmp = make_temp_dir("src-override");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::Project),
        "api.default_model should be attributed to project layer"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn source_inherits_show_base() {
    let tmp = make_temp_dir("src-inherit");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-sonnet-4-6"
max_retries = 5
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.max_retries"),
        Some(&ConfigLayer::User),
        "api.max_retries should be attributed to user (inherited, not overridden)"
    );
    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::Project),
        "api.default_model should be attributed to project (overridden)"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn format_sources_readable() {
    let tmp = make_temp_dir("src-format");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-sonnet-4-6"
max_retries = 5
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");
    let output = format_sources(&sources);

    assert!(!output.is_empty(), "format_sources should produce non-empty output");
    // Should contain layer names and dotted key paths
    assert!(
        output.contains("user") || output.contains("User"),
        "output should mention 'user' layer: {output}"
    );
    assert!(
        output.contains("project") || output.contains("Project"),
        "output should mention 'project' layer: {output}"
    );
    assert!(
        output.contains("api.default_model"),
        "output should contain dotted key path: {output}"
    );
    cleanup_temp_dir(&tmp);
}

// ===========================================================================
// 5. setting_sources filter
// ===========================================================================

#[test]
fn filter_user_only() {
    let tmp = make_temp_dir("filter-user");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();
    fs::write(
        archon_dir.join("config.local.toml"),
        "[api]\ndefault_model = \"claude-haiku-3-6\"\n",
    )
    .unwrap();

    let filter = vec![ConfigLayer::User];
    let config = load_layered_config(Some(&user_cfg), &work, None, Some(&filter))
        .expect("filtered load should succeed");
    assert_eq!(
        config.api.default_model, "claude-sonnet-4-6",
        "only user layer should be loaded when filter=[User]"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn filter_user_and_project() {
    let tmp = make_temp_dir("filter-user-proj");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-6\"\n",
    )
    .unwrap();
    fs::write(
        archon_dir.join("config.local.toml"),
        "[api]\ndefault_model = \"claude-haiku-3-6\"\n",
    )
    .unwrap();

    let filter = vec![ConfigLayer::User, ConfigLayer::Project];
    let config = load_layered_config(Some(&user_cfg), &work, None, Some(&filter))
        .expect("filtered load should succeed");
    assert_eq!(
        config.api.default_model, "claude-opus-4-6",
        "local layer should be skipped when filter=[User, Project]"
    );
    cleanup_temp_dir(&tmp);
}
