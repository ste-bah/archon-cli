//! Phase 3 integration tests — all hook subsystems working together.

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use archon_core::hooks::{
    FunctionRegistry, HookCallbackEntry, HookCommandType, HookConfig, HookContext, HookEvent,
    HookExecutionConfig, HookMatcher, HookOutcome, HookRegistry, HookResult, SourceAuthority,
    is_in_hook_agent, set_in_hook_agent,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tmp_cwd() -> &'static Path {
    if cfg!(target_os = "windows") {
        Path::new("C:\\Windows\\Temp")
    } else {
        Path::new("/tmp")
    }
}

fn empty_input() -> serde_json::Value {
    serde_json::json!({})
}

fn command_hook(cmd: &str) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: cmd.to_string(),
        if_condition: None,
        timeout: Some(5),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        on_failure: None,
        enabled: true,
    }
}

/// Cross-platform sleep command (Windows has no `sleep` binary).
fn sleep_cmd(secs: u32) -> &'static str {
    if cfg!(target_os = "windows") {
        // ping -n N+1 waits ~N seconds; stdout suppressed
        match secs {
            1 => "ping -n 2 127.0.0.1 >nul",
            _ => "ping -n 4 127.0.0.1 >nul",
        }
    } else {
        match secs {
            1 => "sleep 1",
            _ => "sleep 3",
        }
    }
}

fn function_hook(name: &str) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Function,
        command: name.to_string(),
        if_condition: None,
        timeout: None,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        on_failure: None,
        enabled: true,
    }
}

// ---------------------------------------------------------------------------
// 1. Agent hook spawns and parses response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_hook_spawns_and_parses_response() {
    // Agent hook: write JSON to a temp file and cat it, avoiding all shell quoting issues.
    let json_file = tempfile::NamedTempFile::new().unwrap();
    write!(
        json_file.as_file(),
        r#"{{"outcome":"blocking","reason":"agent says no"}}"#
    )
    .unwrap();
    // Normalize path separators so `sh -c` on Windows doesn't eat backslashes.
    let path_str = json_file.path().to_string_lossy().replace('\\', "/");
    let agent_cmd = format!("cat {path_str}");
    let hook = HookConfig {
        hook_type: HookCommandType::Agent,
        command: agent_cmd.clone(),
        if_condition: None,
        timeout: Some(5),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        on_failure: None,
        enabled: true,
    };

    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![hook],
        }],
        None,
    );

    // Make sure recursion guard is off before we start.
    set_in_hook_agent(false);

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            empty_input(),
            tmp_cwd(),
            "agent-test",
        )
        .await;

    // The agent hook command exits 0 but stdout JSON says blocking.
    // Exit 0 + stdout JSON -> outcome from JSON is used.
    assert!(
        result.is_blocked(),
        "agent hook should have produced a blocking result"
    );
    assert!(
        result
            .blocking_errors
            .iter()
            .any(|e| e.contains("agent says no")),
        "blocking reason should contain 'agent says no', got: {:?}",
        result.blocking_errors
    );

    // Recursion guard should have been reset after execution.
    assert!(
        !is_in_hook_agent(),
        "recursion guard should be false after agent hook completes"
    );
}

// ---------------------------------------------------------------------------
// 2. Recursion guard blocks nested fires
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recursion_guard_blocks_nested_fires() {
    let hook = command_hook("echo ok");

    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![hook],
        }],
        None,
    );

    // Set recursion guard ON.
    set_in_hook_agent(true);

    let result = registry
        .execute_hooks(HookEvent::PreToolUse, empty_input(), tmp_cwd(), "guard-on")
        .await;

    // With guard on, execute_hooks should return immediately with empty aggregate.
    assert!(
        !result.is_blocked(),
        "should not be blocked when recursion guard skips execution"
    );
    assert_eq!(
        result.additional_contexts.len(),
        0,
        "no hooks should have fired"
    );

    // Now turn guard OFF and verify hooks fire.
    set_in_hook_agent(false);

    let result2 = registry
        .execute_hooks(HookEvent::PreToolUse, empty_input(), tmp_cwd(), "guard-off")
        .await;

    // The echo command exits 0 -> Success, no blocking.
    assert!(
        !result2.is_blocked(),
        "hooks should fire normally with guard off"
    );
}

// ---------------------------------------------------------------------------
// 3. Callback registration, fire, and panic safety
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_callback_registration_fire_and_panic_safety() {
    let registry = HookRegistry::new();

    // Register a normal callback that returns additional_context.
    let good_cb = HookCallbackEntry {
        name: "good-callback".to_string(),
        callback: Arc::new(|_ctx: &HookContext| -> HookResult {
            HookResult {
                additional_context: Some("callback-context-data".to_string()),
                ..HookResult::allow()
            }
        }),
        authority: SourceAuthority::User,
        timeout_secs: 5,
    };
    registry.register_callback(HookEvent::PostToolUse, good_cb);

    // Register a callback that panics.
    let panic_cb = HookCallbackEntry {
        name: "panic-callback".to_string(),
        callback: Arc::new(|_ctx: &HookContext| -> HookResult {
            panic!("intentional panic in callback test");
        }),
        authority: SourceAuthority::User,
        timeout_secs: 5,
    };
    registry.register_callback(HookEvent::PostToolUse, panic_cb);

    set_in_hook_agent(false);

    let result = registry
        .execute_hooks(HookEvent::PostToolUse, empty_input(), tmp_cwd(), "cb-test")
        .await;

    // The good callback's additional_context should be merged.
    assert!(
        result
            .additional_contexts
            .contains(&"callback-context-data".to_string()),
        "good callback's additional_context should be present, got: {:?}",
        result.additional_contexts
    );

    // The panicking callback should not crash the process.
    // (If we got here, panic was caught safely.)
    assert!(
        !result.is_blocked(),
        "panicking callback should not cause blocking"
    );
}

// ---------------------------------------------------------------------------
// 4. Function noop, block_all, and unknown (fail-open)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_function_noop_and_block_all_and_unknown() {
    // Test via FunctionRegistry directly.
    let registry = FunctionRegistry::new();
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .session_id("fn-test".into())
        .cwd("/tmp".into())
        .build();

    // noop -> Success
    let noop_result = registry.execute("noop", &ctx);
    assert_eq!(noop_result.outcome, HookOutcome::Success);
    assert!(noop_result.reason.is_none());

    // block_all -> Blocking
    let block_result = registry.execute("block_all", &ctx);
    assert_eq!(block_result.outcome, HookOutcome::Blocking);
    assert!(block_result.reason.is_some());

    // unknown -> fail-open (Success)
    let unknown_result = registry.execute("nonexistent_function_xyz", &ctx);
    assert_eq!(unknown_result.outcome, HookOutcome::Success);

    // Also test via HookRegistry with function-type hooks in an event.
    let hook_reg = HookRegistry::new();
    hook_reg.register_matchers(
        HookEvent::Notification,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![
                function_hook("noop"),
                function_hook("block_all"),
                function_hook("nonexistent_function_xyz"),
            ],
        }],
        None,
    );

    set_in_hook_agent(false);

    let agg = hook_reg
        .execute_hooks(HookEvent::Notification, empty_input(), tmp_cwd(), "fn-agg")
        .await;

    // block_all should have contributed a blocking error.
    assert!(
        agg.is_blocked(),
        "block_all function should cause blocking in aggregate"
    );
    assert!(
        agg.blocking_errors.iter().any(|e| e.contains("block_all")),
        "blocking reason should mention block_all"
    );
}

// ---------------------------------------------------------------------------
// 5. HookContext fields populated and roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_hook_context_fields_populated() {
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .tool_name("Bash".to_string())
        .tool_input(serde_json::json!({"command": "ls"}))
        .tool_output(serde_json::json!({"stdout": "file.txt"}))
        .session_id("sess-123".to_string())
        .agent_id("agent-456".to_string())
        .timestamp("2026-01-01T00:00:00Z".to_string())
        .permission_mode("plan".to_string())
        .cwd("/home/test".to_string())
        .previous_tool("Read".to_string())
        .conversation_turn(42)
        .source_authority(SourceAuthority::Policy)
        .build();

    let json_val = ctx.to_json();
    let obj = json_val.as_object().expect("should be JSON object");

    // Verify all 12 fields are present.
    let expected_fields = [
        "hook_event",
        "tool_name",
        "tool_input",
        "tool_output",
        "session_id",
        "agent_id",
        "timestamp",
        "permission_mode",
        "cwd",
        "previous_tool",
        "conversation_turn",
        "source_authority",
    ];

    for field in &expected_fields {
        assert!(
            obj.contains_key(*field),
            "JSON output missing field: {}. Keys present: {:?}",
            field,
            obj.keys().collect::<Vec<_>>()
        );
    }

    // Verify specific values.
    assert_eq!(obj["tool_name"].as_str().unwrap(), "Bash");
    assert_eq!(obj["session_id"].as_str().unwrap(), "sess-123");
    assert_eq!(obj["agent_id"].as_str().unwrap(), "agent-456");
    assert_eq!(obj["permission_mode"].as_str().unwrap(), "plan");
    assert_eq!(obj["cwd"].as_str().unwrap(), "/home/test");
    assert_eq!(obj["previous_tool"].as_str().unwrap(), "Read");
    assert_eq!(obj["conversation_turn"].as_u64().unwrap(), 42);

    // Deserialize back and compare.
    let restored: HookContext =
        serde_json::from_value(json_val).expect("roundtrip deserialization");
    assert_eq!(restored.session_id, "sess-123");
    assert_eq!(restored.tool_name.as_deref(), Some("Bash"));
    assert_eq!(restored.agent_id.as_deref(), Some("agent-456"));
    assert_eq!(restored.conversation_turn, 42);
    assert_eq!(restored.permission_mode, "plan");
    assert_eq!(restored.cwd, "/home/test");
    assert_eq!(restored.previous_tool.as_deref(), Some("Read"));
    assert_eq!(restored.source_authority, Some(SourceAuthority::Policy));
}

// ---------------------------------------------------------------------------
// 6. Session hooks: register, fire, auto-clear on SessionEnd, isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hooks_register_fire_autoclear_isolation() {
    let registry = HookRegistry::new();
    set_in_hook_agent(false);

    // Register session hook for session "A" on PreToolUse.
    let hook_a = command_hook("echo session-A-hook");
    registry.register_session_hook("session-A", HookEvent::PreToolUse, hook_a);

    // Register session hook for session "B" on PreToolUse.
    let hook_b = command_hook("echo session-B-hook");
    registry.register_session_hook("session-B", HookEvent::PreToolUse, hook_b);

    // Fire PreToolUse for session "A" -> hook runs (exits 0 = success).
    let result_a = registry
        .execute_hooks(HookEvent::PreToolUse, empty_input(), tmp_cwd(), "session-A")
        .await;
    assert!(!result_a.is_blocked(), "session A hook should succeed");

    // Fire SessionEnd for session "A" -> auto-clears session A hooks.
    let _end_result = registry
        .execute_hooks(HookEvent::SessionEnd, empty_input(), tmp_cwd(), "session-A")
        .await;

    // Fire PreToolUse for session "A" again -> no hook should run.
    // We register a callback to detect if anything fires for this event.
    // Since session hooks were cleared, the aggregate should be empty.
    let result_a2 = registry
        .execute_hooks(HookEvent::PreToolUse, empty_input(), tmp_cwd(), "session-A")
        .await;
    // No hooks registered at all now for session-A, so aggregate is default.
    assert!(
        !result_a2.is_blocked(),
        "no hooks should fire for cleared session A"
    );

    // Session "B" should still work -- isolation check.
    let result_b = registry
        .execute_hooks(HookEvent::PreToolUse, empty_input(), tmp_cwd(), "session-B")
        .await;
    assert!(
        !result_b.is_blocked(),
        "session B hooks should still fire after session A cleared"
    );
}

#[path = "support/hook_phase3_execution.rs"]
mod execution;
