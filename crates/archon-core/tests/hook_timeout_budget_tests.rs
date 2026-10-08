//! The legacy aggregate key supplies a fresh fallback idle window per hook.
use archon_core::hooks::{
    HookCallbackEntry, HookConfig, HookEvent, HookExecutionConfig, HookMatcher, HookRegistry,
    HookResult,
};
use std::sync::Arc;

fn hook(command: &str, timeout: Option<u32>) -> HookConfig {
    serde_json::from_value(
        serde_json::json!({"type": "command", "command": command, "timeout": timeout}),
    )
    .unwrap()
}

fn registry(ms: u64) -> HookRegistry {
    HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: ms,
    })
}

async fn execute(registry: &HookRegistry) -> archon_core::hooks::AggregatedHookResult {
    let dir = tempfile::tempdir().unwrap();
    registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            dir.path(),
            "idle",
        )
        .await
}

#[tokio::test]
async fn explicit_hook_window_overrides_zero_aggregate_fallback() {
    let registry = registry(0);
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![hook("echo ok", Some(60))],
        }],
        None,
    );
    let result = execute(&registry).await;
    // A zero fallback would be an expired window: PreToolUse would block.
    assert!(!result.is_blocked(), "{:?}", result.block_reason());
    assert!(result.nonblocking_errors.is_empty(), "{result:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn later_hooks_and_session_hooks_get_fresh_windows() {
    let registry = registry(1000);
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![
                hook(&finished_after("sleep 0.6", "first"), None),
                hook(&finished_after("sleep 0.6", "second"), None),
                hook(&finished_after("true", "third"), None),
            ],
        }],
        None,
    );
    registry.register_session_hook(
        "idle",
        HookEvent::PreToolUse,
        hook(&finished_after("true", "session"), None),
    );
    let result = execute(&registry).await;
    assert!(!result.is_blocked(), "{:?}", result.block_reason());
    // Every hook ran to completion, in order: none was skipped or cut short.
    assert_eq!(
        result.additional_contexts,
        vec!["first", "second", "third", "session"]
    );
}

#[cfg(unix)]
fn finished_after(work: &str, marker: &str) -> String {
    format!(r#"{work}; printf '%s' '{{"outcome":"success","additional_context":"{marker}"}}'"#)
}

#[tokio::test]
async fn callback_is_not_skipped_by_aggregate_clock() {
    let registry = registry(0);
    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "completion".to_owned(),
            authority: archon_core::hooks::SourceAuthority::User,
            timeout_secs: 60,
            callback: Arc::new(|_| HookResult::block("callback ran".to_owned())),
        },
    );
    let result = execute(&registry).await;
    assert!(
        result
            .block_reason()
            .unwrap_or_default()
            .contains("callback ran")
    );
}

#[test]
fn test_aggregate_timeout_budget_default_is_30s() {
    assert_eq!(HookExecutionConfig::default().aggregate_timeout_ms, 30_000);
}

#[test]
fn test_hook_execution_config_serialization() {
    let config: HookExecutionConfig =
        serde_json::from_str(r#"{"aggregate_timeout_ms":15000}"#).unwrap();
    assert_eq!(config.aggregate_timeout_ms, 15_000);
    assert_eq!(
        serde_json::to_value(config).unwrap()["aggregate_timeout_ms"],
        15_000
    );
}

#[cfg(unix)]
async fn registry_stall(
    event: HookEvent,
    policy: Option<archon_core::hooks::HookFailurePolicy>,
    timeout: Option<u32>,
    fallback: u64,
) {
    use archon_core::hooks::HookFailurePolicy;
    let registry = registry(fallback);
    let mut stalled = hook("sleep 5", timeout);
    stalled.on_failure = policy;
    stalled.once = Some(true);
    let mut disabled = hook("sleep 5; exit 2", timeout);
    disabled.enabled = false;
    let mut conditional = hook("sleep 5; exit 2", timeout);
    conditional.if_condition = Some("Read".into());
    registry.register_matchers(
        event.clone(),
        vec![HookMatcher {
            matcher: None,
            hooks: vec![disabled, conditional, stalled],
        }],
        None,
    );
    let dir = tempfile::tempdir().unwrap();
    let result = registry
        .execute_hooks(event.clone(), serde_json::json!({}), dir.path(), "idle")
        .await;
    assert!(format!("{result:?}").contains("no progress"), "{result:?}");
    assert_eq!(
        result.is_blocked(),
        policy == Some(HookFailurePolicy::Block)
    );
    assert_eq!(
        result.blocking_errors.len() + result.nonblocking_errors.len(),
        1,
        "ineligible hooks must not run"
    );
    // Only an allowing stop is listed for display; a blocking one blocks.
    assert_eq!(
        result.no_progress_stops,
        if policy == Some(HookFailurePolicy::Block) {
            Vec::new()
        } else {
            result.nonblocking_errors.clone()
        }
    );
    let repeated = registry
        .execute_hooks(event, serde_json::json!({}), dir.path(), "idle")
        .await;
    assert!(
        repeated.blocking_errors.is_empty() && repeated.nonblocking_errors.is_empty(),
        "once hooks must remain ineligible: {repeated:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn registry_preserves_post_tool_no_progress() {
    registry_stall(HookEvent::PostToolUse, None, Some(1), 30_000).await;
}

#[cfg(unix)]
#[tokio::test]
async fn registry_preserves_explicitly_allowing_pre_tool_no_progress() {
    registry_stall(
        HookEvent::PreToolUse,
        Some(archon_core::hooks::HookFailurePolicy::Allow),
        Some(1),
        30_000,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn registry_preserves_fallback_window_stalls_and_failure_policy() {
    registry_stall(HookEvent::PostToolUse, None, None, 1000).await;
    registry_stall(
        HookEvent::PreToolUse,
        Some(archon_core::hooks::HookFailurePolicy::Block),
        None,
        1000,
    )
    .await;
}

#[tokio::test]
async fn registry_preserves_observational_callback_no_progress() {
    let registry = registry(30_000);
    registry.register_callback(
        HookEvent::PostToolUse,
        HookCallbackEntry {
            name: "stalled".into(),
            authority: archon_core::hooks::SourceAuthority::User,
            timeout_secs: 0,
            callback: Arc::new(|_| HookResult::default()),
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let result = registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            dir.path(),
            "idle",
        )
        .await;
    assert!(!result.is_blocked());
    assert_eq!(result.nonblocking_errors.len(), 1);
    assert!(
        result.nonblocking_errors[0].contains("no progress"),
        "{result:?}"
    );
}
