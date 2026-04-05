//! Tests for the Plugin API — PluginRegistry, adapters (TASK-CLI-302).
//!
//! Tests cover: tool registration/namespacing, hook adapter construction,
//! command registration, unregister_all cleanup, duplicate rejection,
//! invalid schema rejection, enable/disable state.

use archon_plugin::api::{PluginRegistry, PluginRegistryError};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn valid_schema() -> String {
    r#"{"type":"object","properties":{"arg":{"type":"string"}}}"#.to_string()
}

fn invalid_schema() -> String {
    "not-valid-json{".to_string()
}

// ── PluginRegistry construction ───────────────────────────────────────────────

#[test]
fn registry_creates_empty() {
    let reg = PluginRegistry::new();
    assert_eq!(reg.tool_count(), 0);
    assert_eq!(reg.hook_count(), 0);
    assert_eq!(reg.command_count(), 0);
}

// ── Tool registration ─────────────────────────────────────────────────────────

#[test]
fn register_tool_succeeds_with_valid_schema() {
    let mut reg = PluginRegistry::new();
    let result = reg.register_tool("my-plugin", "my_tool", valid_schema(), vec![]);
    assert!(result.is_ok(), "expected Ok, got {result:?}");
}

#[test]
fn registered_tool_name_namespaced_with_colon() {
    let mut reg = PluginRegistry::new();
    reg.register_tool("my-plugin", "my_tool", valid_schema(), vec![])
        .unwrap();
    let tools = reg.tool_names();
    assert!(
        tools.contains(&"my-plugin:my_tool"),
        "expected 'my-plugin:my_tool' in {tools:?}"
    );
}

#[test]
fn register_tool_rejects_invalid_json_schema() {
    let mut reg = PluginRegistry::new();
    let result = reg.register_tool("my-plugin", "bad_tool", invalid_schema(), vec![]);
    assert!(
        matches!(result, Err(PluginRegistryError::InvalidSchema(_))),
        "expected InvalidSchema, got {result:?}"
    );
}

#[test]
fn register_tool_rejects_duplicate_name() {
    let mut reg = PluginRegistry::new();
    reg.register_tool("my-plugin", "my_tool", valid_schema(), vec![])
        .unwrap();
    let result = reg.register_tool("my-plugin", "my_tool", valid_schema(), vec![]);
    assert!(
        matches!(result, Err(PluginRegistryError::DuplicateTool(_))),
        "expected DuplicateTool, got {result:?}"
    );
}

#[test]
fn register_tool_different_plugins_same_tool_name_ok() {
    // Two plugins registering a tool with the same local name → different namespaced names
    let mut reg = PluginRegistry::new();
    reg.register_tool("plugin-a", "do_thing", valid_schema(), vec![])
        .unwrap();
    let result = reg.register_tool("plugin-b", "do_thing", valid_schema(), vec![]);
    assert!(
        result.is_ok(),
        "expected Ok for different plugin namespaces"
    );
    let names = reg.tool_names();
    assert!(names.contains(&"plugin-a:do_thing"));
    assert!(names.contains(&"plugin-b:do_thing"));
}

#[test]
fn tool_count_increments() {
    let mut reg = PluginRegistry::new();
    assert_eq!(reg.tool_count(), 0);
    reg.register_tool("p", "t1", valid_schema(), vec![])
        .unwrap();
    assert_eq!(reg.tool_count(), 1);
    reg.register_tool("p", "t2", valid_schema(), vec![])
        .unwrap();
    assert_eq!(reg.tool_count(), 2);
}

// ── Hook registration ─────────────────────────────────────────────────────────

#[test]
fn register_hook_succeeds() {
    let mut reg = PluginRegistry::new();
    let result = reg.register_hook("my-plugin", "PreToolUse", "echo hook-fired");
    assert!(result.is_ok(), "expected Ok, got {result:?}");
}

#[test]
fn register_hook_rejects_unknown_event() {
    let mut reg = PluginRegistry::new();
    let result = reg.register_hook("my-plugin", "NotAnEvent", "echo nope");
    assert!(
        matches!(result, Err(PluginRegistryError::InvalidHookEvent(_))),
        "expected InvalidHookEvent, got {result:?}"
    );
}

#[test]
fn hook_count_increments() {
    let mut reg = PluginRegistry::new();
    reg.register_hook("p", "PreToolUse", "echo a").unwrap();
    reg.register_hook("p", "PostToolUse", "echo b").unwrap();
    assert_eq!(reg.hook_count(), 2);
}

#[test]
fn hook_adapter_has_command_string() {
    let mut reg = PluginRegistry::new();
    reg.register_hook("my-plugin", "PreToolUse", "my-hook-cmd --arg val")
        .unwrap();
    let hooks = reg.hook_entries();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].command, "my-hook-cmd --arg val");
}

#[test]
fn hook_adapter_has_no_priority_field() {
    // HookConfig does not have a priority field — ordering is by plugin load order
    let mut reg = PluginRegistry::new();
    reg.register_hook("p", "PreToolUse", "cmd").unwrap();
    let hooks = reg.hook_entries();
    // Just verify it compiles and returns — no priority assertion needed
    assert_eq!(hooks.len(), 1);
}

// ── Command registration ──────────────────────────────────────────────────────

#[test]
fn register_command_succeeds() {
    let mut reg = PluginRegistry::new();
    let result = reg.register_command("my-plugin", "my-cmd");
    assert!(result.is_ok());
}

#[test]
fn register_command_rejects_duplicate() {
    let mut reg = PluginRegistry::new();
    reg.register_command("p", "cmd").unwrap();
    let result = reg.register_command("p", "cmd");
    assert!(
        matches!(result, Err(PluginRegistryError::DuplicateCommand(_))),
        "expected DuplicateCommand, got {result:?}"
    );
}

#[test]
fn command_count_increments() {
    let mut reg = PluginRegistry::new();
    reg.register_command("p", "cmd1").unwrap();
    reg.register_command("p", "cmd2").unwrap();
    assert_eq!(reg.command_count(), 2);
}

// ── unregister_all ────────────────────────────────────────────────────────────

#[test]
fn unregister_all_removes_all_registrations() {
    let mut reg = PluginRegistry::new();
    reg.register_tool("p", "t1", valid_schema(), vec![])
        .unwrap();
    reg.register_hook("p", "PreToolUse", "cmd").unwrap();
    reg.register_command("p", "c1").unwrap();
    assert_eq!(reg.tool_count(), 1);
    assert_eq!(reg.hook_count(), 1);
    assert_eq!(reg.command_count(), 1);

    reg.unregister_all("p");
    assert_eq!(reg.tool_count(), 0);
    assert_eq!(reg.hook_count(), 0);
    assert_eq!(reg.command_count(), 0);
}

#[test]
fn unregister_all_only_removes_target_plugin() {
    let mut reg = PluginRegistry::new();
    reg.register_tool("plugin-a", "tool_a", valid_schema(), vec![])
        .unwrap();
    reg.register_tool("plugin-b", "tool_b", valid_schema(), vec![])
        .unwrap();
    reg.register_hook("plugin-a", "PreToolUse", "a-cmd")
        .unwrap();
    reg.register_hook("plugin-b", "PostToolUse", "b-cmd")
        .unwrap();

    reg.unregister_all("plugin-a");

    let names = reg.tool_names();
    assert!(
        !names.contains(&"plugin-a:tool_a"),
        "plugin-a tool should be gone"
    );
    assert!(
        names.contains(&"plugin-b:tool_b"),
        "plugin-b tool should remain"
    );
    assert_eq!(reg.hook_count(), 1, "plugin-b hook should remain");
}

#[test]
fn unregister_all_allows_re_registration() {
    let mut reg = PluginRegistry::new();
    reg.register_tool("p", "t", valid_schema(), vec![]).unwrap();
    reg.unregister_all("p");
    // Should succeed since the previous registration was cleared
    let result = reg.register_tool("p", "t", valid_schema(), vec![]);
    assert!(
        result.is_ok(),
        "re-registration after unregister_all should succeed"
    );
}

// ── Enable / disable ──────────────────────────────────────────────────────────

#[test]
fn plugin_enabled_by_default() {
    let reg = PluginRegistry::new();
    // A plugin not explicitly set is treated as enabled
    assert!(reg.is_enabled("some-plugin"));
}

#[test]
fn set_plugin_disabled() {
    let mut reg = PluginRegistry::new();
    reg.set_enabled("my-plugin", false);
    assert!(!reg.is_enabled("my-plugin"));
}

#[test]
fn set_plugin_re_enabled() {
    let mut reg = PluginRegistry::new();
    reg.set_enabled("my-plugin", false);
    reg.set_enabled("my-plugin", true);
    assert!(reg.is_enabled("my-plugin"));
}

// ── Plugin source ID format ───────────────────────────────────────────────────

#[test]
fn plugin_source_id_format() {
    // Source ID is `{name}@{marketplace}` format
    let id = archon_plugin::api::plugin_source_id("my-plugin", "official");
    assert_eq!(id, "my-plugin@official");
}

#[test]
fn plugin_source_id_default_marketplace() {
    let id = archon_plugin::api::plugin_source_id("my-plugin", "local");
    assert_eq!(id, "my-plugin@local");
}

// ── Tool adapter as Box<dyn Tool> ─────────────────────────────────────────────

#[test]
fn plugin_tool_adapter_implements_tool_trait() {
    use archon_plugin::adapter_tool::PluginToolAdapter;
    use archon_tools::tool::Tool;
    let adapter = PluginToolAdapter::new(
        "my-plugin".to_string(),
        "my_tool".to_string(),
        "Does something".to_string(),
        valid_schema(),
    );
    let boxed: Box<dyn Tool> = Box::new(adapter);
    assert_eq!(boxed.name(), "my-plugin:my_tool");
    assert_eq!(boxed.description(), "Does something");
}

#[test]
fn plugin_tool_adapter_name_uses_colon_separator() {
    use archon_plugin::adapter_tool::PluginToolAdapter;
    use archon_tools::tool::Tool;
    let adapter = PluginToolAdapter::new(
        "ns".to_string(),
        "the_tool".to_string(),
        "desc".to_string(),
        valid_schema(),
    );
    assert_eq!(adapter.name(), "ns:the_tool");
}

#[test]
fn plugin_tool_adapter_input_schema_is_json() {
    use archon_plugin::adapter_tool::PluginToolAdapter;
    use archon_tools::tool::Tool;
    let adapter = PluginToolAdapter::new(
        "p".to_string(),
        "t".to_string(),
        "d".to_string(),
        valid_schema(),
    );
    let schema = adapter.input_schema();
    assert!(schema.is_object(), "input_schema should be JSON object");
}

#[test]
fn plugin_tool_adapter_permission_level_is_risky() {
    use archon_plugin::adapter_tool::PluginToolAdapter;
    use archon_tools::tool::{PermissionLevel, Tool};
    let adapter = PluginToolAdapter::new(
        "p".to_string(),
        "t".to_string(),
        "d".to_string(),
        valid_schema(),
    );
    let input = serde_json::json!({});
    assert_eq!(adapter.permission_level(&input), PermissionLevel::Risky);
}

// ── Hook adapter ──────────────────────────────────────────────────────────────

#[test]
fn hook_adapter_produces_hook_config() {
    use archon_core::hooks::HookCommandType;
    use archon_plugin::adapter_hook::PluginHookAdapter;
    let adapter = PluginHookAdapter::new(
        "my-plugin".to_string(),
        "PreToolUse".to_string(),
        "my-script --flag".to_string(),
    );
    let config = adapter.to_hook_config();
    assert_eq!(config.command, "my-script --flag");
    assert!(matches!(config.hook_type, HookCommandType::Command));
}

#[test]
fn hook_adapter_event_name() {
    use archon_plugin::adapter_hook::PluginHookAdapter;
    let adapter = PluginHookAdapter::new(
        "p".to_string(),
        "SessionStart".to_string(),
        "cmd".to_string(),
    );
    assert_eq!(adapter.event(), "SessionStart");
}

// ── Command adapter ───────────────────────────────────────────────────────────

#[test]
fn command_adapter_namespaced_name() {
    use archon_plugin::adapter_command::PluginCommandAdapter;
    let adapter = PluginCommandAdapter::new("my-plugin".to_string(), "my-cmd".to_string());
    assert_eq!(adapter.namespaced_name(), "my-plugin:my-cmd");
}

#[test]
fn command_adapter_plugin_id() {
    use archon_plugin::adapter_command::PluginCommandAdapter;
    let adapter = PluginCommandAdapter::new("p".to_string(), "cmd".to_string());
    assert_eq!(adapter.plugin_id(), "p");
}

// ── tools_from_plugin_instance (TASK-CLI-500 Fix 2) ──────────────────────────

/// WAT module that registers one tool during init (via start function) and
/// supports archon_call_tool dispatch. The tool name is "my_tool".
///
/// Tool name bytes "my_tool" at offset 100 (len=7).
/// Schema bytes {"type":"object","properties":{}} at offset 200 (len=31).
fn registering_dispatch_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "archon" "archon_register_tool" (func $reg (param i32 i32 i32 i32)))
            (memory (export "memory") 2)
            (global $top (mut i32) (i32.const 1024))
            (data (i32.const 100) "my_tool")
            (data (i32.const 200) "{\"type\":\"object\",\"properties\":{}}")
            (func $init
                (call $reg (i32.const 100) (i32.const 7) (i32.const 200) (i32.const 31))
            )
            (start $init)
            (func (export "alloc") (param $n i32) (result i32)
                (local $ret i32)
                (local.set $ret (global.get $top))
                (global.set $top (i32.add (global.get $top) (local.get $n)))
                (local.get $ret))
            (func (export "dealloc") (param i32 i32))
            (func (export "archon_guest_api_version") (result i32) i32.const 1)
            (func (export "archon_call_tool")
                (param i32 i32 i32 i32 i32 i32) (result i32)
                i32.const 0)
        )"#,
    )
    .expect("WAT parse failed")
}

/// Minimal WAT with archon_call_tool but no tool registration (start function absent).
fn dispatch_only_wasm() -> Vec<u8> {
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
                (i32.store8 (local.get $rp) (i32.const 123))
                (i32.store8 (i32.add (local.get $rp) (i32.const 1)) (i32.const 125))
                (i32.store (local.get $rlp) (i32.const 2))
                i32.const 0)
        )"#,
    )
    .expect("WAT parse failed")
}

#[test]
fn tools_from_plugin_instance_empty_for_no_registered_tools() {
    use archon_plugin::{
        api::tools_from_plugin_instance,
        host::{PluginHostConfig, WasmPluginHost},
    };
    use std::sync::{Arc, Mutex};

    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-api-tools-empty")
        .join(ts_str());
    std::fs::create_dir_all(&tmp).unwrap();
    // Load a module that registers no tools.
    let instance = host
        .load_plugin(dispatch_only_wasm(), vec![], Some("p"), tmp)
        .unwrap();
    let host_arc = Arc::new(Mutex::new(host));

    let tools = tools_from_plugin_instance("p", &instance, Arc::clone(&host_arc));
    assert!(tools.is_empty(), "no registered tools → empty vec");
}

#[test]
fn tools_from_plugin_instance_creates_adapter_per_registered_tool() {
    use archon_plugin::{
        api::tools_from_plugin_instance,
        capability::PluginCapability,
        host::{PluginHostConfig, WasmPluginHost},
    };
    use archon_tools::tool::Tool;
    use std::sync::{Arc, Mutex};

    let mut host = WasmPluginHost::new(PluginHostConfig::default()).unwrap();
    let tmp = std::env::temp_dir()
        .join("archon-api-tools-adapters")
        .join(ts_str());
    std::fs::create_dir_all(&tmp).unwrap();
    // Load the module that registers "my_tool" during init.
    let instance = host
        .load_plugin(
            registering_dispatch_wasm(),
            vec![PluginCapability::ToolRegister],
            Some("my-plugin"),
            tmp,
        )
        .unwrap();
    let host_arc = Arc::new(Mutex::new(host));

    let tools = tools_from_plugin_instance("my-plugin", &instance, Arc::clone(&host_arc));
    assert_eq!(tools.len(), 1, "expected one adapter for 'my_tool'");
    assert_eq!(tools[0].name(), "my-plugin:my_tool");
}

fn ts_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        .to_string()
}
