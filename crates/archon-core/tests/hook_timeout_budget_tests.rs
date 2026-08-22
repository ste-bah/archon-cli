/// TASK-HOOK-031: Aggregate Timeout Budget tests
///
/// Tests cover:
/// - HookExecutionConfig default aggregate_timeout_ms is 30_000
/// - AggregatedHookResult default skipped_count is 0
/// - Skipped count incremented on budget exhaustion
/// - Fast hooks all complete within budget (skipped_count stays 0)
/// - Per-hook timeout clamped to remaining budget
/// - Budget-exhausted hooks apply their configured/event-default failure policy
/// - HookExecutionConfig serialization round-trip
use archon_core::hooks::{
    AggregatedHookResult, HookCommandType, HookConfig, HookEvent, HookExecutionConfig,
    HookFailurePolicy, HookMatcher, HookRegistry,
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
        on_failure: None,
        enabled: true,
    }
}

fn matcher_with_hooks(hooks: Vec<HookConfig>) -> HookMatcher {
    HookMatcher {
        matcher: None,
        hooks,
    }
}

/// A leading hook whose only job is to burn a 1ms aggregate budget so that the
/// hook registered after it is budget-skipped.
///
/// It declares `Allow` so it cannot contribute to `is_blocked()`, because the
/// tests below assert about the *second* hook and this one must not be able to
/// answer for it. That matters: `HookConfig.timeout` has second granularity, so
/// a sub-millisecond remaining budget clamps this hook's 5s timeout to the 1s
/// floor, and a shell spawn on a loaded machine can exceed 1s. When it does the
/// hook is cut short by the budget and — on a gating event — the default policy
/// blocks, which is correct behaviour but is not the fact under test.
fn budget_burner() -> HookConfig {
    let mut hook = cmd_hook("sleep 0.01", Some(5));
    hook.on_failure = Some(HookFailurePolicy::Allow);
    hook
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

/// Which deadline wins when a hook asks for more time than the aggregate budget
/// has left: the budget.
///
/// The claim is about the *outcome*, not the duration. This used to assert
/// `elapsed.as_secs() < 4` around a 2s budget, which is a measurement of how
/// loaded the machine is — it was observed failing at 5.7s while other builds
/// ran and passing on a quiet re-run, in both cases against a clamp that worked.
/// Worse, a machine where the shell could not spawn at all would fail in
/// milliseconds and sail through the old assertion.
///
/// A `sleep 5` under a 2s budget and a 60s hook timeout can only end two ways,
/// and they are distinguishable without a clock: the clamp holds and the hook is
/// killed by its deadline (`RunError::Timeout` → gating-event block policy), or
/// the clamp is gone, 60s wins, and the sleep exits 0 into a clean Success. The
/// wall clock never enters into it — on a machine slow enough to take 5.7s, this
/// still reports a timeout, because the sleep is longer than the budget by
/// construction.
///
/// The clamp arithmetic itself is covered without any process at all in
/// `hooks::registry::budget`.
#[tokio::test]
async fn test_per_hook_timeout_clamped_to_remaining_budget() {
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

    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;

    assert_eq!(
        result.skipped_count, 0,
        "the hook must have been started and then cut short, not skipped before it \
         ran; a skipped hook would prove nothing about the per-hook clamp"
    );
    let reason = result.block_reason().unwrap_or_default();
    assert!(
        reason.contains("timed out"),
        "expected the hook to be killed by the clamped 2s budget rather than run to \
         completion under its own 60s timeout, but the reported outcome was \
         {reason:?} (blocked: {})",
        result.is_blocked()
    );
}

// ---------------------------------------------------------------------------
// test_budget_exhausted_applies_default_failure_policy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_budget_exhausted_applies_default_failure_policy() {
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 1,
    };
    let registry = HookRegistry::with_config(config);

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            budget_burner(),
            cmd_hook("exit 0", Some(5)),
        ])],
        None,
    );

    let input = serde_json::json!({"tool_name": "Bash"});
    let cwd = PathBuf::from("/tmp");
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;

    assert!(result.skipped_count > 0);
    assert!(
        result.is_blocked(),
        "budget-exhausted PreToolUse hooks must use the default block policy"
    );
}

#[tokio::test]
async fn test_budget_exhausted_respects_explicit_allow_policy() {
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 1,
    };
    let registry = HookRegistry::with_config(config);
    let mut skipped_hook = cmd_hook("exit 0", Some(5));
    skipped_hook.on_failure = Some(HookFailurePolicy::Allow);

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![budget_burner(), skipped_hook])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &PathBuf::from("/tmp"),
            "test-session",
        )
        .await;

    assert!(result.skipped_count > 0);
    assert!(!result.is_blocked(), "blocked: {:?}", result.block_reason());
}

#[tokio::test]
async fn test_budget_exhaustion_does_not_apply_policy_to_non_matching_hook() {
    let registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 1,
    });
    let mut non_matching = cmd_hook("exit 0", Some(5));
    non_matching.if_condition = Some("Read".to_string());

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![budget_burner(), non_matching])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &PathBuf::from("/tmp"),
            "test-session",
        )
        .await;

    assert!(!result.is_blocked(), "blocked: {:?}", result.block_reason());
    assert_eq!(
        result.skipped_count, 0,
        "a hook that does not match is ineligible, not timeout-skipped"
    );
}

#[tokio::test]
async fn test_observational_budget_exhaustion_remains_non_blocking() {
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 1,
    };
    let registry = HookRegistry::with_config(config);

    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("sleep 0.01", Some(5)),
            cmd_hook("exit 0", Some(5)),
        ])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &PathBuf::from("/tmp"),
            "test-session",
        )
        .await;

    assert!(result.skipped_count > 0);
    assert!(!result.is_blocked());
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
