/// TASK-HOOK-031: Aggregate Timeout Budget tests
///
/// Tests cover:
/// - HookExecutionConfig default aggregate_timeout_ms is 30_000
/// - AggregatedHookResult default skipped_count is 0
/// - Skipped count incremented on budget exhaustion
/// - Fast hooks all complete within budget (skipped_count stays 0)
/// - Per-hook timeout clamped to remaining budget
/// - Budget-exhausted hooks return Success (fail-open)
/// - HookExecutionConfig serialization round-trip
use archon_core::hooks::{
    AggregatedHookResult, HookCommandType, HookConfig, HookEvent, HookExecutionConfig, HookMatcher,
    HookRegistry,
};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helper: build a HookConfig for a shell command
// ---------------------------------------------------------------------------

fn cmd_hook(command: &str, timeout: Option<u32>) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: command.to_string(),
        if_condition: None,
        timeout,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: HashMap::new(),
        allowed_env_vars: Vec::new(),
        enabled: true,
    }
}

fn matcher_with_hooks(hooks: Vec<HookConfig>) -> HookMatcher {
    HookMatcher {
        matcher: None,
        hooks,
    }
}

// ---------------------------------------------------------------------------
// test_aggregate_timeout_budget_default_is_30s
// ---------------------------------------------------------------------------

#[test]
fn test_aggregate_timeout_budget_default_is_30s() {
    let config = HookExecutionConfig::default();
    assert_eq!(config.aggregate_timeout_ms, 30_000);
}

// ---------------------------------------------------------------------------
// test_skipped_count_starts_at_zero
// ---------------------------------------------------------------------------

#[test]
fn test_skipped_count_starts_at_zero() {
    let result = AggregatedHookResult::new();
    assert_eq!(result.skipped_count, 0);
}

// ---------------------------------------------------------------------------
// test_skipped_count_incremented_on_budget_exhaustion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_skipped_count_incremented_on_budget_exhaustion() {
    // Create a registry with a tiny budget (1ms).
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 1,
    };
    let registry = HookRegistry::with_config(config);

    // Register 3 hooks: one fast, two slow.
    // The fast one might complete, but the slow ones should be skipped
    // once the 1ms budget is exhausted.
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("sleep 1", Some(10)),
            cmd_hook("sleep 1", Some(10)),
            cmd_hook("sleep 1", Some(10)),
        ])],
        None,
    );

    let input = serde_json::json!({"tool_name": "Bash"});
    let cwd = PathBuf::from("/tmp");
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;

    // With a 1ms budget, at least some hooks should be skipped.
    assert!(
        result.skipped_count > 0,
        "Expected skipped_count > 0 with 1ms budget, got {}",
        result.skipped_count
    );
}

// ---------------------------------------------------------------------------
// test_fast_hooks_all_complete_within_budget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fast_hooks_all_complete_within_budget() {
    // Default budget (30s) with fast hooks.
    let registry = HookRegistry::new();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("echo ok", Some(5)),
            cmd_hook("echo ok", Some(5)),
            cmd_hook("echo ok", Some(5)),
        ])],
        None,
    );

    let input = serde_json::json!({"tool_name": "Bash"});
    let cwd = PathBuf::from("/tmp");
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;

    assert_eq!(
        result.skipped_count, 0,
        "Fast hooks should all complete within budget"
    );
}

// ---------------------------------------------------------------------------
// test_per_hook_timeout_clamped_to_remaining_budget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_per_hook_timeout_clamped_to_remaining_budget() {
    // Budget = 2 seconds. Hook has timeout=60s but command sleeps 5s.
    // The hook should be killed at ~2s (clamped to remaining budget), not 60s.
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 2_000,
    };
    let registry = HookRegistry::with_config(config);

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![cmd_hook("sleep 5", Some(60))])],
        None,
    );

    let input = serde_json::json!({"tool_name": "Bash"});
    let cwd = PathBuf::from("/tmp");

    let start = std::time::Instant::now();
    let _result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;
    let elapsed = start.elapsed();

    // Should complete in roughly 2s (budget), NOT 5s (sleep) or 60s (hook timeout).
    assert!(
        elapsed.as_secs() < 4,
        "Expected hook to be killed within ~2s budget, but took {:.1}s",
        elapsed.as_secs_f64()
    );
}

// ---------------------------------------------------------------------------
// test_budget_exhausted_returns_success_failopen
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_budget_exhausted_returns_success_failopen() {
    // Budget = 1ms so hooks get skipped.
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 1,
    };
    let registry = HookRegistry::with_config(config);

    // Register a hook that would block (exit 2) if it ran.
    // But since budget is exhausted, it should be skipped with Success (fail-open).
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("sleep 0.01", Some(5)), // first hook eats the budget
            cmd_hook("exit 2", Some(5)),     // this would block, but should be skipped
        ])],
        None,
    );

    let input = serde_json::json!({"tool_name": "Bash"});
    let cwd = PathBuf::from("/tmp");
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;

    // The blocking hook was skipped, so result should NOT be blocked.
    assert!(
        !result.is_blocked(),
        "Skipped hooks should fail-open (not block)"
    );
}

// ---------------------------------------------------------------------------
// test_hook_execution_config_serialization
// ---------------------------------------------------------------------------

#[test]
fn test_hook_execution_config_serialization() {
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 15_000,
    };

    let json = serde_json::to_string(&config).expect("serialize");
    let deserialized: HookExecutionConfig = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(deserialized.aggregate_timeout_ms, 15_000);
}
