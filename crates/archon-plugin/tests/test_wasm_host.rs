//! Tests for the WASM plugin host (TASK-CLI-301 + TASK-CLI-500).
//!
//! Tests cover: host initialization, capability checking, error types,
//! manifest parsing, invalid WASM rejection, valid minimal WASM loading,
//! ABI version negotiation, and WASM tool dispatch.

use std::path::PathBuf;

use archon_plugin::{
    capability::{CapabilityChecker, PluginCapability},
    error::PluginError,
    host::{PluginHostConfig, WasmPluginHost},
    types::{PluginManifest, PluginMetadata},
};

// ── WAT helpers ───────────────────────────────────────────────────────────────

/// Minimal valid WASM plugin: exports alloc, dealloc, archon_guest_api_version, memory.
/// Does NOT export archon_call_tool — used for pre-dispatch tests.
fn minimal_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param i32) (result i32) i32.const 0)
            (func (export "dealloc") (param i32 i32))
            (func (export "archon_guest_api_version") (result i32) i32.const 1)
        )"#,
    )
    .expect("WAT parse failed")
}

/// WASM with a bump allocator and a working archon_call_tool implementation.
/// archon_call_tool writes "{}" to result_ptr and length=2 to result_len_ptr.
fn dispatch_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 2)
            (global $top (mut i32) (i32.const 1024))
            (func (export "alloc") (param $n i32) (result i32)
                (local $ret i32)
                (local.set $ret (global.get $top))
                (global.set $top (i32.add (global.get $top) (local.get $n)))
                (local.get $ret))
            (func (export "dealloc") (param i32 i32))
            (func (export "archon_guest_api_version") (result i32) i32.const 1)
            (func (export "archon_call_tool")
                (param $np i32) (param $nl i32)
                (param $ap i32) (param $al i32)
                (param $rp i32) (param $rlp i32) (result i32)
                ;; Write "{}" to result_ptr
                (i32.store8 (local.get $rp) (i32.const 123))
                (i32.store8 (i32.add (local.get $rp) (i32.const 1)) (i32.const 125))
                ;; Write length=2 as LE i32 to result_len_ptr
                (i32.store (local.get $rlp) (i32.const 2))
                i32.const 0)
        )"#,
    )
    .expect("WAT parse failed")
}

/// WASM with no exports (invalid for a plugin).
fn no_exports_wasm() -> Vec<u8> {
    wat::parse_str("(module)").expect("WAT parse failed")
}

/// Not WASM at all — just garbage bytes.
fn garbage_bytes() -> Vec<u8> {
    b"this is not wasm".to_vec()
}

/// WASM with a version mismatch (reports v999 guest API).
fn version_mismatch_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param i32) (result i32) i32.const 0)
            (func (export "dealloc") (param i32 i32))
            (func (export "archon_guest_api_version") (result i32) i32.const 999)
        )"#,
    )
    .expect("WAT parse failed")
}

/// WASM with a too-old version (reports v0 guest API).
fn old_version_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param i32) (result i32) i32.const 0)
            (func (export "dealloc") (param i32 i32))
            (func (export "archon_guest_api_version") (result i32) i32.const 0)
        )"#,
    )
    .expect("WAT parse failed")
}

// ── PluginError tests ─────────────────────────────────────────────────────────

#[test]
fn plugin_error_load_failed() {
    let e = PluginError::LoadFailed("bad module".into());
    assert!(e.to_string().contains("bad module"));
}

#[test]
fn plugin_error_abi_mismatch() {
    let e = PluginError::AbiMismatch {
        expected: 1,
        got: 99,
    };
    assert!(e.to_string().contains("1"));
    assert!(e.to_string().contains("99"));
}

#[test]
fn plugin_error_capability_denied() {
    let e = PluginError::CapabilityDenied(PluginCapability::Network(vec!["example.com".into()]));
    assert!(e.to_string().contains("capability") || e.to_string().contains("Network"));
}

#[test]
fn plugin_error_timeout() {
    let e = PluginError::Timeout {
        fuel_exhausted: true,
    };
    assert!(e.to_string().len() > 0);
}

#[test]
fn plugin_error_memory_violation() {
    let e = PluginError::MemoryViolation {
        requested: 128 * 1024 * 1024,
        limit: 64 * 1024 * 1024,
    };
    assert!(e.to_string().len() > 0);
}

#[test]
fn plugin_error_manifest_parse() {
    let e = PluginError::ManifestParseError {
        path: PathBuf::from("/tmp/plugin.json"),
        reason: "missing field".into(),
    };
    assert!(e.to_string().contains("plugin.json") || e.to_string().contains("missing field"));
}

#[test]
fn plugin_error_manifest_validation() {
    let e = PluginError::ManifestValidationError {
        path: PathBuf::from("/tmp/plugin.json"),
        fields: vec!["name".into(), "version".into()],
    };
    assert!(e.to_string().len() > 0);
}

#[test]
fn plugin_error_dependency_unsatisfied() {
    let e = PluginError::DependencyUnsatisfied {
        plugin: "my-plugin".into(),
        dependency: "other-plugin@1.0".into(),
    };
    assert!(e.to_string().contains("my-plugin") || e.to_string().contains("other-plugin"));
}

#[test]
fn plugin_error_component_load_failed() {
    let e = PluginError::ComponentLoadFailed {
        path: PathBuf::from("/tmp/plugin.wasm"),
        reason: "link error".into(),
    };
    assert!(e.to_string().contains("plugin.wasm") || e.to_string().contains("link error"));
}

// ── PluginCapability tests ────────────────────────────────────────────────────

#[test]
fn plugin_capability_all_variants_exist() {
    let caps: Vec<PluginCapability> = vec![
        PluginCapability::None,
        PluginCapability::ReadFs(vec![PathBuf::from("/tmp")]),
        PluginCapability::WriteFs(vec![PathBuf::from("/tmp")]),
        PluginCapability::Network(vec!["example.com".into()]),
        PluginCapability::ToolRegister,
        PluginCapability::HookRegister,
        PluginCapability::CommandRegister,
        PluginCapability::LspRegister,
        PluginCapability::DataDirWrite,
    ];
    assert_eq!(caps.len(), 9);
}

// ── CapabilityChecker tests ───────────────────────────────────────────────────

#[test]
fn checker_denies_fs_read_when_not_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::None]);
    assert!(!checker.can_read_fs(&PathBuf::from("/tmp/file.txt")));
}

#[test]
fn checker_allows_fs_read_when_granted_matching_path() {
    let checker =
        CapabilityChecker::new(vec![PluginCapability::ReadFs(vec![PathBuf::from("/tmp")])]);
    assert!(checker.can_read_fs(&PathBuf::from("/tmp/file.txt")));
}

#[test]
fn checker_denies_fs_read_for_non_matching_path() {
    let checker = CapabilityChecker::new(vec![PluginCapability::ReadFs(vec![PathBuf::from(
        "/allowed",
    )])]);
    assert!(!checker.can_read_fs(&PathBuf::from("/forbidden/file.txt")));
}

#[test]
fn checker_denies_network_when_not_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::None]);
    assert!(!checker.can_use_network("example.com"));
}

#[test]
fn checker_allows_network_for_granted_host() {
    let checker =
        CapabilityChecker::new(vec![PluginCapability::Network(vec!["example.com".into()])]);
    assert!(checker.can_use_network("example.com"));
}

#[test]
fn checker_denies_tool_register_when_not_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::None]);
    assert!(!checker.can_register_tool());
}

#[test]
fn checker_allows_tool_register_when_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::ToolRegister]);
    assert!(checker.can_register_tool());
}

#[test]
fn checker_allows_hook_register_when_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::HookRegister]);
    assert!(checker.can_register_hook());
}

#[test]
fn checker_denies_hook_register_when_not_granted() {
    let checker = CapabilityChecker::new(vec![PluginCapability::None]);
    assert!(!checker.can_register_hook());
}

// ── PluginManifest parsing tests ──────────────────────────────────────────────

#[test]
fn manifest_parses_valid_json() {
    let json = r#"{
        "name": "my-plugin",
        "version": "1.0.0",
        "description": "A test plugin",
        "author": "Test Author",
        "capabilities": []
    }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.name, "my-plugin");
    assert_eq!(manifest.version, "1.0.0");
}

#[test]
fn manifest_requires_name() {
    let json = r#"{ "version": "1.0.0" }"#;
    let result: Result<PluginManifest, _> = serde_json::from_str(json);
    assert!(result.is_err());
}

#[test]
fn manifest_requires_version() {
    let json = r#"{ "name": "test" }"#;
    let result: Result<PluginManifest, _> = serde_json::from_str(json);
    assert!(result.is_err());
}

#[test]
fn manifest_optional_fields_default() {
    let json = r#"{ "name": "test", "version": "0.1.0" }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert!(manifest.description.is_none());
    assert!(manifest.dependencies.is_empty());
    assert!(manifest.capabilities.is_empty());
}

#[test]
fn manifest_to_metadata() {
    let json = r#"{ "name": "test-plugin", "version": "2.0.0" }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    let meta = PluginMetadata::from_manifest(&manifest);
    assert_eq!(meta.name, "test-plugin");
    assert_eq!(meta.version, "2.0.0");
}

// ── WasmPluginHost initialization tests ──────────────────────────────────────

#[test]
fn host_initializes_with_default_config() {
    let config = PluginHostConfig::default();
    let host = WasmPluginHost::new(config);
    assert!(host.is_ok(), "host should initialize: {:?}", host.err());
}

#[test]
fn host_initializes_with_custom_limits() {
    let config = PluginHostConfig {
        max_memory_bytes: 32 * 1024 * 1024,
        fuel_budget: 5_000_000,
        ..Default::default()
    };
    let host = WasmPluginHost::new(config);
    assert!(host.is_ok());
}

// ── Instance loading tests ────────────────────────────────────────────────────

#[test]
fn host_rejects_garbage_bytes() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let result = host.load_plugin(garbage_bytes(), vec![], None, std::env::temp_dir());
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), PluginError::LoadFailed(_)));
}

#[test]
fn host_rejects_module_with_no_exports() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let result = host.load_plugin(no_exports_wasm(), vec![], None, std::env::temp_dir());
    assert!(result.is_err());
}

#[test]
fn host_loads_minimal_valid_wasm() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-plugin-test")
        .join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    let result = host.load_plugin(minimal_wasm(), vec![], None, tmp);
    assert!(
        result.is_ok(),
        "should load minimal WASM: {:?}",
        result.err()
    );
}

#[test]
fn host_creates_data_dir_on_load() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let data_dir = std::env::temp_dir()
        .join("archon-plugin-data-test")
        .join(uuid_str());
    let result = host.load_plugin(minimal_wasm(), vec![], None, data_dir.clone());
    assert!(result.is_ok());
    assert!(data_dir.exists(), "data_dir should be created");
    let instance = result.unwrap();
    assert_eq!(instance.data_dir(), &data_dir);
}

#[test]
fn host_rejects_version_mismatch_too_new() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let tmp = std::env::temp_dir().join("archon-test").join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    let result = host.load_plugin(version_mismatch_wasm(), vec![], None, tmp);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PluginError::AbiMismatch { .. }
    ));
}

#[test]
fn host_rejects_version_mismatch_too_old() {
    let config = PluginHostConfig::default();
    let mut host = WasmPluginHost::new(config).unwrap();
    let tmp = std::env::temp_dir().join("archon-test").join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    let result = host.load_plugin(old_version_wasm(), vec![], None, tmp);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PluginError::AbiMismatch { .. }
    ));
}

// ── WASM dispatch tests (TASK-CLI-500 Fix 1) ──────────────────────────────────

#[test]
fn dispatch_tool_without_runtime_returns_error_json() {
    // Dispatch before any plugin is loaded → well-formed error JSON, no panic.
    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let result = host.dispatch_tool("some_tool", "{}");
    let parsed: serde_json::Value = serde_json::from_str(&result)
        .expect("dispatch_tool must return valid JSON even without runtime");
    assert!(
        parsed.get("error").is_some(),
        "expected 'error' key: {result}"
    );
}

#[test]
fn dispatch_tool_without_archon_call_tool_export_returns_error_json() {
    // minimal_wasm has no archon_call_tool → error JSON, no panic.
    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-dispatch-test")
        .join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    host.load_plugin(minimal_wasm(), vec![], Some("test-plugin"), tmp)
        .unwrap();

    let result = host.dispatch_tool("my_tool", r#"{"arg":"value"}"#);
    let parsed: serde_json::Value = serde_json::from_str(&result)
        .expect("dispatch_tool must return valid JSON when export missing");
    assert!(
        parsed.get("error").is_some(),
        "expected 'error' key: {result}"
    );
}

#[test]
fn dispatch_tool_with_archon_call_tool_returns_json() {
    // dispatch_wasm exports archon_call_tool and writes "{}" as result.
    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-dispatch-ok")
        .join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    host.load_plugin(dispatch_wasm(), vec![], Some("dispatch-plugin"), tmp)
        .unwrap();

    let result = host.dispatch_tool("my_tool", r#"{"input":"test"}"#);
    // The guest writes "{}" — verify we get valid JSON back.
    let parsed: serde_json::Value = serde_json::from_str(&result)
        .unwrap_or_else(|e| panic!("dispatch_tool returned non-JSON: {result:?} — {e}"));
    // The result is whatever the guest returned; it must be a JSON value.
    let _ = parsed;
}

#[test]
fn dispatch_tool_does_not_panic_on_empty_args() {
    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-dispatch-empty")
        .join(uuid_str());
    std::fs::create_dir_all(&tmp).unwrap();
    host.load_plugin(dispatch_wasm(), vec![], Some("p"), tmp)
        .unwrap();
    // Must not panic regardless of empty args.
    let _ = host.dispatch_tool("tool", "");
}

#[test]
fn dispatch_tool_returns_string_on_load_failure() {
    // Loading fails (garbage bytes) — dispatch_tool must still not panic.
    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let _ = host.load_plugin(garbage_bytes(), vec![], None, std::env::temp_dir());
    // load failed, runtime is None → should return error JSON.
    let result = host.dispatch_tool("tool", "{}");
    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("error path must produce valid JSON");
    assert!(parsed.get("error").is_some());
}

// ── ABI / valid hook event tests ──────────────────────────────────────────────

#[test]
fn abi_valid_hook_events_count() {
    use archon_plugin::abi::VALID_HOOK_EVENTS;
    assert_eq!(VALID_HOOK_EVENTS.len(), 20);
}

#[test]
fn abi_hook_events_contain_pre_tool_use() {
    use archon_plugin::abi::VALID_HOOK_EVENTS;
    assert!(VALID_HOOK_EVENTS.contains(&"PreToolUse"));
}

#[test]
fn abi_hook_events_contain_session_start() {
    use archon_plugin::abi::VALID_HOOK_EVENTS;
    assert!(VALID_HOOK_EVENTS.contains(&"SessionStart"));
}

#[test]
fn abi_hook_events_contain_worktree_events() {
    use archon_plugin::abi::VALID_HOOK_EVENTS;
    assert!(VALID_HOOK_EVENTS.contains(&"WorktreeCreate"));
    assert!(VALID_HOOK_EVENTS.contains(&"WorktreeRemove"));
}

// ── host/guest API version constants ─────────────────────────────────────────

#[test]
fn host_api_version_is_1() {
    use archon_plugin::abi::{HOST_API_VERSION, MIN_SUPPORTED_GUEST_VERSION};
    assert_eq!(HOST_API_VERSION, 1);
    assert_eq!(MIN_SUPPORTED_GUEST_VERSION, 1);
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn uuid_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{n:x}")
}
