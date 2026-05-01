//! Integration tests for Function hook type (HookCommandType::Function).

use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};
use std::collections::HashMap;

/// Helper to build a minimal HookConfig with Function type.
fn function_hook(command: &str) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Function,
        command: command.to_string(),
        if_condition: None,
        timeout: None,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: HashMap::new(),
        allowed_env_vars: Vec::new(),
        enabled: true,
    }
}

#[tokio::test]
async fn test_function_noop_returns_success() {
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![function_hook("noop")],
        }],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-fn-1",
        )
        .await;

    assert!(
        !result.is_blocked(),
        "noop function should not block: {:?}",
        result.blocking_errors
    );
}

#[tokio::test]
async fn test_function_block_all_returns_blocking() {
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![function_hook("block_all")],
        }],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-fn-2",
        )
        .await;

    assert!(
        result.is_blocked(),
        "block_all function should block execution"
    );
    assert!(
        result.block_reason().unwrap().contains("block_all"),
        "block reason should mention block_all"
    );
}

#[tokio::test]
async fn test_function_unknown_returns_success_failopen() {
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![function_hook("nonexistent_fn")],
        }],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-fn-3",
        )
        .await;

    assert!(
        !result.is_blocked(),
        "unknown function should fail-open (not block)"
    );
}

#[tokio::test]
async fn test_function_hook_type_serialization() {
    let json = serde_json::to_string(&HookCommandType::Function).unwrap();
    assert_eq!(json, "\"function\"");

    let deserialized: HookCommandType = serde_json::from_str("\"function\"").unwrap();
    assert_eq!(deserialized, HookCommandType::Function);
}

#[tokio::test]
async fn test_function_hook_via_registry() {
    // Full E2E: register via settings JSON, execute through registry.
    let settings_json = r#"{
        "hooks": {
            "SessionStart": [
                {
                    "hooks": [
                        { "type": "function", "command": "noop" }
                    ]
                }
            ]
        }
    }"#;

    let registry = HookRegistry::load_from_settings_json(settings_json).unwrap();

    let result = registry
        .execute_hooks(
            HookEvent::SessionStart,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-fn-4",
        )
        .await;

    assert!(!result.is_blocked(), "noop via registry should succeed");
}

#[tokio::test]
async fn test_multiple_function_hooks() {
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![function_hook("noop"), function_hook("block_all")],
        }],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-fn-5",
        )
        .await;

    assert!(
        result.is_blocked(),
        "block_all should cause blocking even when noop runs first"
    );
    assert!(
        result.block_reason().unwrap().contains("block_all"),
        "reason should reference block_all"
    );
}
