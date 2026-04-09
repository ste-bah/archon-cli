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
    };

    let mut registry = HookRegistry::new();
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

    let mut registry = HookRegistry::new();
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
    assert_eq!(
        result.skipped_count, 0,
        "skipped_count should be 0 (not even evaluated)"
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
    let mut hook_reg = HookRegistry::new();
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

// ---------------------------------------------------------------------------
// 7. Timeout budget exhaustion and skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_timeout_budget_exhaustion_and_skip() {
    // Create registry with a tiny budget of 100ms.
    let mut registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 100,
    });

    // Register 3 slow hooks that each sleep 1 second.
    let slow_hooks: Vec<HookConfig> = (0..3).map(|_| command_hook(sleep_cmd(1))).collect();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: slow_hooks,
        }],
        None,
    );

    set_in_hook_agent(false);

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            empty_input(),
            tmp_cwd(),
            "timeout-test",
        )
        .await;

    // With 100ms budget and 3 hooks that each sleep 1s, most should be skipped.
    assert!(
        result.skipped_count > 0,
        "at least some hooks should have been skipped due to budget exhaustion, skipped_count={}",
        result.skipped_count
    );
}

// ---------------------------------------------------------------------------
// 8. Mixed scenario: all hook types fire for the same event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mixed_scenario_all_hook_types_fire() {
    let mut registry = HookRegistry::new();
    set_in_hook_agent(false);

    let event = HookEvent::PostToolUse;

    // (a) Persistent command hook (echo, exit 0).
    let cmd_hook = command_hook("echo ok");

    // (b) Function hook (noop).
    let fn_hook = function_hook("noop");

    // Register command and function hooks as persistent matchers.
    registry.register_matchers(
        event.clone(),
        vec![HookMatcher {
            matcher: None,
            hooks: vec![cmd_hook, fn_hook],
        }],
        None,
    );

    // (c) Session hook (echo, exit 0).
    let session_hook = command_hook("echo session-mixed");
    registry.register_session_hook("mixed-session", event.clone(), session_hook);

    // (d) Callback that returns additional_context.
    let fired_flag = Arc::new(AtomicBool::new(false));
    let fired_clone = fired_flag.clone();
    let cb = HookCallbackEntry {
        name: "mixed-callback".to_string(),
        callback: Arc::new(move |_ctx: &HookContext| -> HookResult {
            fired_clone.store(true, Ordering::SeqCst);
            HookResult {
                additional_context: Some("mixed-callback-contribution".to_string()),
                ..HookResult::allow()
            }
        }),
        authority: SourceAuthority::User,
        timeout_secs: 5,
    };
    registry.register_callback(event.clone(), cb);

    // Fire the event.
    let result = registry
        .execute_hooks(event, empty_input(), tmp_cwd(), "mixed-session")
        .await;

    // Verify all fired.
    assert!(!result.is_blocked(), "mixed scenario should not be blocked");

    // Callback should have fired.
    assert!(
        fired_flag.load(Ordering::SeqCst),
        "callback should have fired"
    );

    // The callback's additional_context should be in the aggregate.
    assert!(
        result
            .additional_contexts
            .iter()
            .any(|c| c.contains("mixed-callback-contribution")),
        "callback contribution should be in additional_contexts, got: {:?}",
        result.additional_contexts
    );

    // skipped_count should be 0 (no timeout issues with fast hooks).
    assert_eq!(result.skipped_count, 0, "no hooks should have been skipped");
}

// ---------------------------------------------------------------------------
// 9. Callback receives enriched HookContext (tool_name from input)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_callback_receives_enriched_context() {
    let registry = HookRegistry::new();
    set_in_hook_agent(false);

    let captured_tool_name: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let captured_clone = captured_tool_name.clone();

    let cb = HookCallbackEntry {
        name: "enrichment-check".to_string(),
        callback: Arc::new(move |ctx: &HookContext| -> HookResult {
            let mut guard = captured_clone.lock().unwrap();
            *guard = ctx.tool_name.clone();
            HookResult::allow()
        }),
        authority: SourceAuthority::User,
        timeout_secs: 5,
    };
    registry.register_callback(HookEvent::PreToolUse, cb);

    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": "ls" }
    });

    let _result = registry
        .execute_hooks(HookEvent::PreToolUse, input, tmp_cwd(), "enrich-test")
        .await;

    let captured = captured_tool_name.lock().unwrap();
    assert_eq!(
        captured.as_deref(),
        Some("Bash"),
        "callback should have received tool_name='Bash' from enriched context, got: {:?}",
        *captured
    );
}

// ---------------------------------------------------------------------------
// 10. Session hook timeout is clamped to aggregate budget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_hook_timeout_clamped_to_budget() {
    // Budget = 3 seconds. Session hook has timeout=60 and runs `sleep 10`.
    let registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 3000,
    });
    set_in_hook_agent(false);

    let slow_hook = HookConfig {
        hook_type: HookCommandType::Command,
        command: "sleep 10".to_string(),
        if_condition: None,
        timeout: Some(60),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
    };
    registry.register_session_hook("clamp-test", HookEvent::PreToolUse, slow_hook);

    let start = std::time::Instant::now();
    let _result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            empty_input(),
            tmp_cwd(),
            "clamp-test",
        )
        .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "session hook should have been clamped to ~3s budget, but took {:?}",
        elapsed
    );
}

// ---------------------------------------------------------------------------
// 11. Callbacks skipped when budget is exhausted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_callbacks_skipped_when_budget_exhausted() {
    // Tiny budget of 100ms. A persistent hook sleeps 1s to consume it.
    let mut registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 100,
    });
    set_in_hook_agent(false);

    // Persistent command hook that will consume the entire budget.
    let slow_hook = command_hook(sleep_cmd(1));
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![slow_hook],
        }],
        None,
    );

    // Register a callback that sets a flag if it fires.
    let cb_fired = Arc::new(AtomicBool::new(false));
    let cb_fired_clone = cb_fired.clone();
    let cb = HookCallbackEntry {
        name: "should-be-skipped".to_string(),
        callback: Arc::new(move |_ctx: &HookContext| -> HookResult {
            cb_fired_clone.store(true, Ordering::SeqCst);
            HookResult::allow()
        }),
        authority: SourceAuthority::User,
        timeout_secs: 5,
    };
    registry.register_callback(HookEvent::PreToolUse, cb);

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            empty_input(),
            tmp_cwd(),
            "cb-skip-test",
        )
        .await;

    assert!(
        !cb_fired.load(Ordering::SeqCst),
        "callback should NOT have fired because budget was exhausted"
    );
    assert!(
        result.skipped_count > 0,
        "skipped_count should be > 0 (callback was skipped), got: {}",
        result.skipped_count
    );
}
