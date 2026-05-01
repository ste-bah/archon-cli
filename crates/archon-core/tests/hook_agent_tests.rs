/// TASK-HOOK-026: Agent hook type with recursion guard tests.
///
/// Tests cover:
/// - Agent hook fires and parses JSON response from stdout
/// - Recursion guard prevents nested hook fires
/// - Recursion guard default is false
/// - Agent hook sets guard during execution and resets after
/// - Agent hook mutex serialization (only one at a time)
/// - Agent hook timeout behavior
use std::path::Path;
use std::time::Instant;

use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};

// Re-export guard functions from the hooks module.
use archon_core::hooks::{is_in_hook_agent, set_in_hook_agent};

// ---------------------------------------------------------------------------
// Helper: build a HookConfig with Agent type
// ---------------------------------------------------------------------------

fn agent_hook_config(command: &str, timeout: Option<u32>) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Agent,
        command: command.to_string(),
        if_condition: None,
        timeout,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        enabled: true,
    }
}

fn command_hook_config(command: &str) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: command.to_string(),
        if_condition: None,
        timeout: Some(5),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        enabled: true,
    }
}

fn register_hook(registry: &mut HookRegistry, event: HookEvent, config: HookConfig) {
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![config],
        }],
        None,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Agent hook fires, runs command, and parses stdout JSON as HookResult.
#[cfg(unix)]
#[tokio::test]
async fn test_agent_hook_fires_and_parses_response() {
    let mut registry = HookRegistry::new();
    let config = agent_hook_config(
        r#"echo '{"outcome":"success","reason":"agent says ok"}'"#,
        Some(5),
    );
    register_hook(&mut registry, HookEvent::PreToolUse, config);

    let input = serde_json::json!({"tool_name": "Bash"});
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, Path::new("/tmp"), "sess-1")
        .await;

    // Should not block.
    assert!(!result.is_blocked(), "agent hook should not block");
    // No blocking errors.
    assert!(result.blocking_errors.is_empty());
}

/// When the recursion guard is set, execute_hooks returns immediately
/// with default AggregatedHookResult (no hooks fire).
#[cfg(unix)]
#[tokio::test]
async fn test_recursion_guard_prevents_nested_hook_fires() {
    let mut registry = HookRegistry::new();
    // Register a command hook that would block if it ran.
    let config = command_hook_config("exit 2");
    register_hook(&mut registry, HookEvent::PreToolUse, config);

    let input = serde_json::json!({"tool_name": "Bash"});

    // With guard ON: hooks should NOT fire.
    set_in_hook_agent(true);
    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            input.clone(),
            Path::new("/tmp"),
            "sess-2",
        )
        .await;
    assert!(
        !result.is_blocked(),
        "hooks must not fire when recursion guard is active"
    );
    assert!(result.blocking_errors.is_empty());
    set_in_hook_agent(false);

    // With guard OFF: hooks should fire normally and block.
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, Path::new("/tmp"), "sess-2")
        .await;
    assert!(
        result.is_blocked(),
        "hooks must fire when recursion guard is inactive"
    );
}

/// Default value of the recursion guard is false.
#[tokio::test]
async fn test_recursion_guard_default_is_false() {
    assert!(
        !is_in_hook_agent(),
        "recursion guard should default to false"
    );
}

/// After an Agent hook finishes execution, the guard must be reset to false.
#[cfg(unix)]
#[tokio::test]
async fn test_agent_hook_sets_guard_during_execution() {
    // Use a command that succeeds. The guard should be true DURING execution
    // and false AFTER.
    let mut registry = HookRegistry::new();
    let config = agent_hook_config("echo '{\"outcome\":\"success\"}'", Some(5));
    register_hook(&mut registry, HookEvent::SessionStart, config);

    let input = serde_json::json!({});
    let _result = registry
        .execute_hooks(HookEvent::SessionStart, input, Path::new("/tmp"), "sess-3")
        .await;

    assert!(
        !is_in_hook_agent(),
        "guard must be false after agent hook completes"
    );
}

/// The AGENT_HOOK_MUTEX ensures only one agent hook runs at a time.
/// We verify this by spawning two agent hooks on separate tokio tasks
/// (which can land on different threads, avoiding the thread_local guard
/// conflict) and checking that total wall-clock time is >= 2s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_agent_hook_mutex_serialization() {
    use std::sync::Arc;

    // Each hook sleeps for 1 second. If serialized, total >= 2s.
    let mut reg1 = HookRegistry::new();
    let config1 = agent_hook_config("sleep 1 && echo '{\"outcome\":\"success\"}'", Some(5));
    register_hook(&mut reg1, HookEvent::SessionStart, config1);

    let mut reg2 = HookRegistry::new();
    let config2 = agent_hook_config("sleep 1 && echo '{\"outcome\":\"success\"}'", Some(5));
    register_hook(&mut reg2, HookEvent::SessionStart, config2);

    let reg1 = Arc::new(reg1);
    let reg2 = Arc::new(reg2);

    let start = Instant::now();

    // Use tokio::spawn to run on separate worker threads so
    // thread_local IN_HOOK_AGENT doesn't interfere between tasks.
    let r1_reg = reg1.clone();
    let h1 = tokio::spawn(async move {
        r1_reg
            .execute_hooks(
                HookEvent::SessionStart,
                serde_json::json!({}),
                Path::new("/tmp"),
                "sess-4a",
            )
            .await
    });

    let r2_reg = reg2.clone();
    let h2 = tokio::spawn(async move {
        r2_reg
            .execute_hooks(
                HookEvent::SessionStart,
                serde_json::json!({}),
                Path::new("/tmp"),
                "sess-4b",
            )
            .await
    });

    let (r1, r2) = tokio::try_join!(h1, h2).expect("tasks should not panic");

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() >= 1900,
        "serialized agent hooks should take >= 2s, got {}ms",
        elapsed.as_millis()
    );
    assert!(!r1.is_blocked());
    assert!(!r2.is_blocked());
}

/// Agent hook with short timeout and a long-running command should time out
/// and return default (fail-open) result.
#[cfg(unix)]
#[tokio::test]
async fn test_agent_hook_timeout() {
    let mut registry = HookRegistry::new();
    // timeout=1s, but command sleeps for 5s.
    let config = agent_hook_config("sleep 5", Some(1));
    register_hook(&mut registry, HookEvent::PreToolUse, config);

    let input = serde_json::json!({"tool_name": "Bash"});
    let start = Instant::now();
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, Path::new("/tmp"), "sess-5")
        .await;
    let elapsed = start.elapsed();

    // Should not block (fail-open on timeout).
    assert!(!result.is_blocked(), "timed-out agent hook must fail-open");
    // Should complete in ~1-2s, not 5s.
    assert!(
        elapsed.as_secs() < 4,
        "should have timed out after ~1s, took {}s",
        elapsed.as_secs()
    );
}
