//! Tests for the Plugin Loader (TASK-CLI-303 + TASK-CLI-500).
//!
//! Tests cover: directory scanning, manifest parsing, capability grant checking,
//! dependency verification, enable/disable state, seed directories, cache,
//! PluginLoadResult error handling, and WASM instantiation.

use std::collections::HashMap;
use std::path::PathBuf;

use archon_plugin::{
    capability::PluginCapability,
    loader::{PluginLoader, instantiate_wasm_plugins},
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn unique_tmp(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ))
}

/// Write a minimal valid plugin into `dir/<name>/`.
fn write_plugin(plugins_dir: &std::path::Path, name: &str, version: &str, capabilities: &[&str]) {
    let plugin_dir = plugins_dir.join(name);
    let manifest_dir = plugin_dir.join(".archon-plugin");
    std::fs::create_dir_all(&manifest_dir).unwrap();

    let caps_json = capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let manifest = format!(
        r#"{{
            "name": "{name}",
            "version": "{version}",
            "description": "Test plugin",
            "capabilities": [{caps_json}]
        }}"#
    );
    std::fs::write(manifest_dir.join("plugin.json"), manifest).unwrap();
}

/// Write a minimal plugin that depends on another plugin.
fn write_plugin_with_deps(plugins_dir: &std::path::Path, name: &str, deps: &[&str]) {
    let plugin_dir = plugins_dir.join(name);
    let manifest_dir = plugin_dir.join(".archon-plugin");
    std::fs::create_dir_all(&manifest_dir).unwrap();

    let deps_json = deps
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let manifest = format!(
        r#"{{
            "name": "{name}",
            "version": "1.0.0",
            "dependencies": [{deps_json}],
            "capabilities": []
        }}"#
    );
    std::fs::write(manifest_dir.join("plugin.json"), manifest).unwrap();
}

// ── PluginLoader construction ─────────────────────────────────────────────────

#[test]
fn loader_creates_plugins_dir_if_missing() {
    let dir = unique_tmp("archon-test-loader-missing");
    assert!(!dir.exists());
    let loader = PluginLoader::new(dir.clone());
    let _result = loader.load_all();
    assert!(dir.exists(), "plugins dir should be created on load_all");
}

#[test]
fn loader_empty_dir_returns_empty_result() {
    let dir = unique_tmp("archon-test-loader-empty");
    std::fs::create_dir_all(&dir).unwrap();
    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert!(result.enabled.is_empty());
    assert!(result.disabled.is_empty());
    assert!(result.errors.is_empty());
}

// ── Directory scanning ────────────────────────────────────────────────────────

#[test]
fn loader_discovers_plugin_with_manifest() {
    let dir = unique_tmp("archon-test-loader-discover");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "test-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir.clone()).with_granted_capabilities(vec![]);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1, "expected 1 enabled plugin");
    assert_eq!(result.enabled[0].manifest.name, "test-plugin");
}

#[test]
fn loader_skips_dir_without_manifest() {
    let dir = unique_tmp("archon-test-loader-no-manifest");
    std::fs::create_dir_all(&dir).unwrap();
    // Create a plugin directory with no .archon-plugin/plugin.json
    std::fs::create_dir_all(dir.join("ghost-plugin")).unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    // Ghost plugin has no manifest — should be skipped silently
    assert!(result.enabled.is_empty());
    assert!(
        result.errors.is_empty(),
        "missing manifest should be silent skip"
    );
}

#[test]
fn loader_handles_invalid_manifest_json() {
    let dir = unique_tmp("archon-test-loader-bad-json");
    std::fs::create_dir_all(&dir).unwrap();
    let plugin_dir = dir.join("bad-plugin");
    let manifest_dir = plugin_dir.join(".archon-plugin");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    std::fs::write(manifest_dir.join("plugin.json"), "{ not json }").unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert!(result.enabled.is_empty());
    assert_eq!(result.errors.len(), 1, "expected 1 error for bad JSON");
}

#[test]
fn loader_handles_manifest_missing_required_fields() {
    let dir = unique_tmp("archon-test-loader-missing-fields");
    std::fs::create_dir_all(&dir).unwrap();
    let plugin_dir = dir.join("bad-plugin");
    let manifest_dir = plugin_dir.join(".archon-plugin");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    // Missing "name" field
    std::fs::write(manifest_dir.join("plugin.json"), r#"{"version": "1.0.0"}"#).unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.errors.len(), 1, "expected 1 error for missing name");
}

#[test]
fn loader_handles_manifest_name_with_spaces() {
    let dir = unique_tmp("archon-test-loader-spaces");
    std::fs::create_dir_all(&dir).unwrap();
    let plugin_dir = dir.join("bad-plugin");
    let manifest_dir = plugin_dir.join(".archon-plugin");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    // Name contains spaces — invalid
    std::fs::write(
        manifest_dir.join("plugin.json"),
        r#"{"name": "bad name", "version": "1.0.0"}"#,
    )
    .unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(
        result.errors.len(),
        1,
        "name with spaces should produce an error"
    );
}

// ── Capability grant checking ─────────────────────────────────────────────────

#[test]
fn loader_loads_plugin_with_granted_capabilities() {
    let dir = unique_tmp("archon-test-loader-caps-ok");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "net-plugin", "1.0.0", &["Network"]);

    let loader =
        PluginLoader::new(dir).with_granted_capabilities(vec![PluginCapability::Network(vec![])]);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
    assert!(result.errors.is_empty());
}

#[test]
fn loader_records_error_for_ungranted_capabilities() {
    let dir = unique_tmp("archon-test-loader-caps-denied");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "net-plugin", "1.0.0", &["Network"]);

    // No granted capabilities
    let loader = PluginLoader::new(dir).with_granted_capabilities(vec![]);
    let result = loader.load_all();
    assert!(result.enabled.is_empty());
    assert_eq!(
        result.errors.len(),
        1,
        "ungranted capability should produce error"
    );
}

#[test]
fn loader_loads_plugin_requesting_no_capabilities() {
    let dir = unique_tmp("archon-test-loader-no-caps");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "simple-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir).with_granted_capabilities(vec![]);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
}

// ── Enable / disable ──────────────────────────────────────────────────────────

#[test]
fn loader_plugin_enabled_by_default() {
    let dir = unique_tmp("archon-test-loader-enabled-default");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "my-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
    assert!(result.disabled.is_empty());
}

#[test]
fn loader_plugin_disabled_via_config() {
    let dir = unique_tmp("archon-test-loader-disabled");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "my-plugin", "1.0.0", &[]);

    let mut enabled_map = HashMap::new();
    enabled_map.insert("my-plugin".to_string(), false);

    let loader = PluginLoader::new(dir).with_enabled_state(enabled_map);
    let result = loader.load_all();
    assert!(result.enabled.is_empty());
    assert_eq!(result.disabled.len(), 1);
}

#[test]
fn loader_multiple_plugins_mixed_enable_state() {
    let dir = unique_tmp("archon-test-loader-mixed");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "plugin-a", "1.0.0", &[]);
    write_plugin(&dir, "plugin-b", "1.0.0", &[]);

    let mut enabled_map = HashMap::new();
    enabled_map.insert("plugin-a".to_string(), true);
    enabled_map.insert("plugin-b".to_string(), false);

    let loader = PluginLoader::new(dir).with_enabled_state(enabled_map);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
    assert_eq!(result.enabled[0].manifest.name, "plugin-a");
    assert_eq!(result.disabled.len(), 1);
}

// ── Dependency verification ───────────────────────────────────────────────────

#[test]
fn loader_loads_plugin_with_satisfied_dependency() {
    let dir = unique_tmp("archon-test-loader-dep-ok");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "base-plugin", "1.0.0", &[]);
    write_plugin_with_deps(&dir, "dep-plugin", &["base-plugin@local"]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    // Both should load; dep-plugin loads after base-plugin
    assert_eq!(result.enabled.len(), 2);
    assert!(result.errors.is_empty());
}

#[test]
fn loader_records_dependency_unsatisfied_error() {
    let dir = unique_tmp("archon-test-loader-dep-missing");
    std::fs::create_dir_all(&dir).unwrap();
    // dep-plugin depends on "missing-dep@local" which isn't present
    write_plugin_with_deps(&dir, "dep-plugin", &["missing-dep@local"]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert!(result.enabled.is_empty());
    assert_eq!(
        result.errors.len(),
        1,
        "unsatisfied dependency should produce error"
    );
}

// ── Data directory creation ───────────────────────────────────────────────────

#[test]
fn loader_creates_data_dir_per_plugin() {
    let dir = unique_tmp("archon-test-loader-data-dir");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "my-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
    let data_dir = &result.enabled[0].data_dir;
    assert!(
        data_dir.exists(),
        "data_dir should be created: {data_dir:?}"
    );
}

// ── Seed directory support ────────────────────────────────────────────────────

#[test]
fn loader_discovers_plugins_in_seed_dir() {
    let main_dir = unique_tmp("archon-test-loader-seed-main");
    let seed_dir = unique_tmp("archon-test-loader-seed-seed");
    std::fs::create_dir_all(&main_dir).unwrap();
    std::fs::create_dir_all(&seed_dir).unwrap();

    // Plugin in seed dir, not main dir
    write_plugin(&seed_dir, "seed-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(main_dir).with_seed_dirs(vec![seed_dir]);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1, "seed plugin should be discovered");
    assert_eq!(result.enabled[0].manifest.name, "seed-plugin");
}

#[test]
fn loader_main_dir_takes_precedence_over_seed() {
    let main_dir = unique_tmp("archon-test-loader-precedence-main");
    let seed_dir = unique_tmp("archon-test-loader-precedence-seed");
    std::fs::create_dir_all(&main_dir).unwrap();
    std::fs::create_dir_all(&seed_dir).unwrap();

    // Same plugin in both — main version wins
    write_plugin(&main_dir, "shared-plugin", "2.0.0", &[]);
    write_plugin(&seed_dir, "shared-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(main_dir).with_seed_dirs(vec![seed_dir]);
    let result = loader.load_all();
    assert_eq!(
        result.enabled.len(),
        1,
        "only one instance of shared-plugin"
    );
    assert_eq!(
        result.enabled[0].manifest.version, "2.0.0",
        "main dir version wins"
    );
}

// ── Fail-open loading ─────────────────────────────────────────────────────────

#[test]
fn loader_continues_after_bad_plugin() {
    let dir = unique_tmp("archon-test-loader-fail-open");
    std::fs::create_dir_all(&dir).unwrap();
    // Bad plugin
    let bad_dir = dir.join("bad-plugin").join(".archon-plugin");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("plugin.json"), "{ invalid }").unwrap();
    // Good plugin
    write_plugin(&dir, "good-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1, "good plugin loads despite bad one");
    assert_eq!(result.errors.len(), 1, "bad plugin produces 1 error");
    assert_eq!(result.enabled[0].manifest.name, "good-plugin");
}

// ── PluginLoadResult ──────────────────────────────────────────────────────────

#[test]
fn load_result_error_contains_plugin_id() {
    let dir = unique_tmp("archon-test-loader-error-id");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_dir = dir.join("error-plugin").join(".archon-plugin");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("plugin.json"), "{ invalid }").unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.errors.len(), 1);
    // Error tuple contains plugin ID as first element
    assert_eq!(result.errors[0].0, "error-plugin");
}

#[test]
fn load_result_errors_are_typed_plugin_errors() {
    use archon_plugin::error::PluginError;
    let dir = unique_tmp("archon-test-loader-typed-errors");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_dir = dir.join("bad-plugin").join(".archon-plugin");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("plugin.json"), "{ invalid }").unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.errors.len(), 1);
    // Error should be a typed PluginError variant, not a string
    assert!(
        matches!(
            &result.errors[0].1,
            PluginError::ManifestParseError { .. } | PluginError::ManifestValidationError { .. }
        ),
        "expected typed PluginError, got {:?}",
        result.errors[0].1
    );
}

// ── Cache ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_stores_and_retrieves_bytes() {
    use archon_plugin::cache::WasmCache;
    let cache_dir = unique_tmp("archon-test-cache");
    let cache = WasmCache::new(cache_dir);
    let bytes = b"fake wasm bytes";
    cache.store("my-plugin", "1.0.0", bytes).unwrap();
    let retrieved = cache.get("my-plugin", "1.0.0");
    assert_eq!(retrieved.unwrap(), bytes);
}

#[test]
fn cache_returns_none_for_unknown_plugin() {
    use archon_plugin::cache::WasmCache;
    let cache_dir = unique_tmp("archon-test-cache-miss");
    let cache = WasmCache::new(cache_dir);
    assert!(cache.get("missing", "1.0.0").is_none());
}

#[test]
fn cache_version_is_separate() {
    use archon_plugin::cache::WasmCache;
    let cache_dir = unique_tmp("archon-test-cache-versions");
    let cache = WasmCache::new(cache_dir);
    cache.store("plugin", "1.0.0", b"v1").unwrap();
    cache.store("plugin", "2.0.0", b"v2").unwrap();
    assert_eq!(cache.get("plugin", "1.0.0").unwrap(), b"v1");
    assert_eq!(cache.get("plugin", "2.0.0").unwrap(), b"v2");
}

// ── instantiate_wasm_plugins (TASK-CLI-500 Fix 1) ────────────────────────────

#[test]
fn instantiate_wasm_plugins_empty_for_no_plugins() {
    let dir = unique_tmp("archon-test-instantiate-empty");
    std::fs::create_dir_all(&dir).unwrap();
    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    let instances = instantiate_wasm_plugins(&result);
    assert!(instances.is_empty(), "no plugins → no instances");
}

#[test]
fn instantiate_wasm_plugins_skips_plugins_without_wasm_path() {
    // A plugin with a valid manifest but no plugin.wasm — should produce no instance.
    let dir = unique_tmp("archon-test-instantiate-no-wasm");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "no-wasm-plugin", "1.0.0", &[]);

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1, "plugin loaded");
    assert!(result.enabled[0].wasm_path.is_none(), "no wasm_path");

    let instances = instantiate_wasm_plugins(&result);
    assert!(instances.is_empty(), "no wasm_path → no instance");
}

#[test]
fn instantiate_wasm_plugins_fail_open_on_invalid_wasm() {
    // Plugin with a wasm_path that points to garbage bytes — should not panic.
    let dir = unique_tmp("archon-test-instantiate-bad-wasm");
    std::fs::create_dir_all(&dir).unwrap();
    write_plugin(&dir, "bad-wasm-plugin", "1.0.0", &[]);
    // Write garbage bytes as plugin.wasm.
    let wasm_path = dir.join("bad-wasm-plugin").join("plugin.wasm");
    std::fs::write(&wasm_path, b"not wasm bytes").unwrap();

    let loader = PluginLoader::new(dir);
    let result = loader.load_all();
    assert_eq!(result.enabled.len(), 1);
    assert!(result.enabled[0].wasm_path.is_some());

    // Must not panic — fail-open means we get an empty map.
    let instances = instantiate_wasm_plugins(&result);
    assert!(
        instances.is_empty(),
        "invalid WASM → no instance, fail-open"
    );
}
