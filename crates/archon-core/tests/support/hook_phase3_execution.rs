use super::*;

// ---------------------------------------------------------------------------
// 7. Independent hook windows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_each_hook_gets_its_own_timeout() {
    // A small fallback must not shorten explicitly configured windows.
    let registry = HookRegistry::with_config(HookExecutionConfig {
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

    // Explicit per-hook windows are independent of aggregate elapsed time:
    // a 100 ms window would stop each 1 s sleep, and PreToolUse would block.
    assert!(!result.is_blocked(), "{:?}", result.block_reason());
    assert!(result.nonblocking_errors.is_empty(), "{result:?}");
}

// ---------------------------------------------------------------------------
// 8. Mixed scenario: all hook types fire for the same event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mixed_scenario_all_hook_types_fire() {
    let registry = HookRegistry::new();
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
    assert!(result.nonblocking_errors.is_empty(), "{result:?}");
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
// 10. Explicit session hook window
// ---------------------------------------------------------------------------

/// Explicit session-hook windows override the legacy fallback.
#[tokio::test]
async fn test_session_hook_timeout_overrides_fallback() {
    // Fallback = 3 seconds; the session hook explicitly asks for 60.
    let registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 3000,
    });
    set_in_hook_agent(false);

    let slow_hook = HookConfig {
        hook_type: HookCommandType::Command,
        command: sleep_cmd(3).to_string(),
        if_condition: None,
        timeout: Some(60),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        on_failure: None,
        enabled: true,
    };
    registry.register_session_hook("clamp-test", HookEvent::PreToolUse, slow_hook);

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            empty_input(),
            tmp_cwd(),
            "clamp-test",
        )
        .await;

    // A 100 ms fallback must not shorten the explicit 60 s window.
    assert!(!result.is_blocked(), "{:?}", result.block_reason());
    assert!(result.nonblocking_errors.is_empty(), "{result:?}");
}

// ---------------------------------------------------------------------------
// 11. Callbacks run after earlier hooks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_callbacks_run_after_earlier_hooks_finish() {
    // A persistent hook outlives the legacy fallback.
    let registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 100,
    });
    set_in_hook_agent(false);

    // The persistent command finishes before the callback starts.
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
        name: "must-run".to_string(),
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

    assert!(cb_fired.load(Ordering::SeqCst), "callback must run");
    assert!(!result.is_blocked(), "{:?}", result.block_reason());
}
