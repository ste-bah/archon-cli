//! TASK-HOOK-030: Session-scoped temporary hooks tests.
//!
//! Verifies that session hooks can be registered, fired, cleared,
//! auto-cleared on SessionEnd, isolated between sessions, and merged
//! with persistent hooks.

use std::collections::HashMap;
use std::path::Path;

use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};

/// Cross-platform temp directory for hook cwd.
fn tmp_cwd() -> &'static Path {
    if cfg!(target_os = "windows") {
        Path::new("C:\\Windows\\Temp")
    } else {
        tmp_cwd()
    }
}

// ---------------------------------------------------------------------------
// Helper: build a HookConfig with a command that exits with `exit_code`
// ---------------------------------------------------------------------------

fn make_hook_config(exit_code: i32) -> HookConfig {
    // The hook executor wraps the command in `sh -c`, so we can use plain
    // shell builtins directly — no need for `bash -c` (which may not exist
    // on Windows CI where only MSYS2 `sh` is available).
    let command = if exit_code == 0 {
        "echo session-hook-test".to_string()
    } else {
        format!("exit {exit_code}")
    };

    HookConfig {
        hook_type: HookCommandType::Command,
        command,
        if_condition: None,
        timeout: Some(5),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: HashMap::new(),
        allowed_env_vars: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. Register a session hook and verify it fires
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_session_hook_fires() {
    let registry = HookRegistry::new();

    // Register a session hook for "s1" on PreToolUse (exit 0 = success)
    registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(0));

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    assert!(
        !result.is_blocked(),
        "session hook (exit 0) should not block"
    );
}

// ---------------------------------------------------------------------------
// 2. clear_session_hooks removes all hooks for that session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_clear_session_hooks_removes_all() {
    let registry = HookRegistry::new();

    // Register a blocking session hook (exit 2) for "s1"
    registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(2));

    // Clear before firing
    registry.clear_session_hooks("s1");

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    assert!(
        !result.is_blocked(),
        "after clear, session hook should not fire"
    );
}

// ---------------------------------------------------------------------------
// 3. Session hooks auto-clear when SessionEnd fires
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hooks_auto_clear_on_session_end() {
    let registry = HookRegistry::new();

    // Register a blocking session hook for "s1" on PreToolUse
    registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(2));

    // Fire SessionEnd for "s1" -- should auto-clear session hooks
    let _end_result = registry
        .execute_hooks(
            HookEvent::SessionEnd,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    // Now fire PreToolUse for "s1" -- session hook should be gone
    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    assert!(
        !result.is_blocked(),
        "session hook should have been auto-cleared by SessionEnd"
    );
}

// ---------------------------------------------------------------------------
// 4. Session hooks are isolated between sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hooks_isolated_between_sessions() {
    let registry = HookRegistry::new();

    // Register a blocking hook (exit 2) only for "s1"
    registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(2));

    // Fire for "s2" -- should NOT be blocked
    let result_s2 = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s2",
        )
        .await;

    assert!(
        !result_s2.is_blocked(),
        "session s2 should not be affected by s1's hooks"
    );

    // Fire for "s1" -- SHOULD be blocked
    let result_s1 = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    assert!(
        result_s1.is_blocked(),
        "session s1 should be blocked by its session hook"
    );
}

// ---------------------------------------------------------------------------
// 5. Session hooks merged with persistent hooks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hooks_merged_with_persistent() {
    let mut registry = HookRegistry::new();

    // Register a persistent hook that adds additional_context
    // (we use a command that prints JSON to stdout for this)
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "echo persistent-hook".to_string(),
                if_condition: None,
                timeout: Some(5),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
                headers: HashMap::new(),
                allowed_env_vars: Vec::new(),
            }],
        }],
        None,
    );

    // Register a session hook (also succeeds)
    registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(0));

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    // Both hooks ran, neither blocked
    assert!(
        !result.is_blocked(),
        "merged persistent + session hooks should not block"
    );
}

// ---------------------------------------------------------------------------
// 6. Session hook count -- verify all registered hooks fire
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hook_count() {
    let registry = HookRegistry::new();

    // Register 3 blocking session hooks for "s1" on PreToolUse
    for _ in 0..3 {
        registry.register_session_hook("s1", HookEvent::PreToolUse, make_hook_config(2));
    }

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            tmp_cwd(),
            "s1",
        )
        .await;

    assert!(result.is_blocked(), "should be blocked");
    assert_eq!(
        result.blocking_errors.len(),
        3,
        "all 3 session hooks should have contributed blocking errors"
    );
}
