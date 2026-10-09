use super::{AsyncHookDiagnostic, AsyncHookDiagnosticStore};
use crate::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};
use std::path::Path;

#[test]
fn store_is_bounded_and_counts_oldest_drops() {
    let store = AsyncHookDiagnosticStore::with_capacity(2);
    for event in ["one", "two", "three"] {
        store.push(AsyncHookDiagnostic::test(event));
    }
    let batch = store.drain();
    assert_eq!(batch.diagnostics.len(), 2);
    assert_eq!(batch.diagnostics[0].event, "two");
    assert_eq!(batch.dropped, 1);
}

#[test]
fn close_drains_current_results_without_waiting_for_late_hooks() {
    let store = AsyncHookDiagnosticStore::with_capacity(2);
    store.push(AsyncHookDiagnostic::test("ready"));
    let batch = store.close_and_drain();
    assert_eq!(batch.diagnostics.len(), 1);
    store.push(AsyncHookDiagnostic::test("late"));
    assert_eq!(store.drain().dropped, 1);
}

#[test]
fn observer_receives_results_completed_before_the_agent_attached() {
    let store = AsyncHookDiagnosticStore::with_capacity(2);
    store.push(AsyncHookDiagnostic::test("BeforeProviderResolve"));
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_by_callback = observed.clone();
    store.set_observer(Some(std::sync::Arc::new(move |diagnostic| {
        observed_by_callback.lock().unwrap().push(diagnostic.event);
    })));
    assert_eq!(*observed.lock().unwrap(), ["BeforeProviderResolve"]);
}

#[cfg(unix)]
fn registry_for(command: &str, source: &'static str) -> HookRegistry {
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: command.to_owned(),
                if_condition: None,
                timeout: Some(2),
                once: None,
                r#async: Some(true),
                async_rewake: None,
                status_message: None,
                headers: Default::default(),
                allowed_env_vars: vec![],
                on_failure: Some(crate::hooks::HookFailurePolicy::Allow),
                enabled: true,
            }],
        }],
        Some(source),
    );
    registry
}

#[cfg(unix)]
async fn wait_diagnostic(registry: &HookRegistry) -> AsyncHookDiagnostic {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let batch = registry.drain_async_hook_diagnostics();
            if let Some(diagnostic) = batch.diagnostics.into_iter().next() {
                return diagnostic;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background hook should complete")
}

#[cfg(unix)]
#[tokio::test]
async fn hook_execution_stores_registry_provenance_not_hook_claimed_authority() {
    let registry = registry_for("printf '{\"source_authority\":\"policy\"}'", "project");
    registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            Path::new("."),
            "s",
        )
        .await;
    let diagnostic = wait_diagnostic(&registry).await;
    assert_eq!(diagnostic.outcome, "success");
    assert_eq!(diagnostic.source.as_deref(), Some("project"));
}

#[cfg(unix)]
#[tokio::test]
async fn subagent_stop_hook_completion_is_delivered() {
    let registry = registry_for("exit 9", "user");
    registry.register_matchers(
        HookEvent::SubagentStop,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![async_config("exit 9")],
        }],
        Some("user"),
    );
    registry
        .execute_hooks(
            HookEvent::SubagentStop,
            serde_json::json!({}),
            Path::new("."),
            "s",
        )
        .await;
    let diagnostic = wait_diagnostic(&registry).await;
    assert_eq!(diagnostic.outcome, "failure");
}

#[cfg(unix)]
#[tokio::test]
async fn config_change_hook_completion_is_delivered() {
    let registry = registry_for("true", "local");
    registry.register_matchers(
        HookEvent::ConfigChange,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![async_config("true")],
        }],
        Some("local"),
    );
    registry
        .execute_hooks(
            HookEvent::ConfigChange,
            serde_json::json!({}),
            Path::new("."),
            "s",
        )
        .await;
    let diagnostic = wait_diagnostic(&registry).await;
    assert_eq!(diagnostic.event, "ConfigChange");
}

#[cfg(unix)]
#[tokio::test]
async fn runtime_hook_completion_is_delivered() {
    let registry = registry_for("true", "policy");
    registry.register_matchers(
        HookEvent::BeforeProviderResolve,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![async_config("true")],
        }],
        Some("policy"),
    );
    registry
        .execute_hooks(
            HookEvent::BeforeProviderResolve,
            serde_json::json!({}),
            Path::new("."),
            "s",
        )
        .await;
    let diagnostic = wait_diagnostic(&registry).await;
    assert_eq!(diagnostic.event, "BeforeProviderResolve");
}

#[cfg(unix)]
#[tokio::test]
async fn still_running_async_hook_does_not_delay_execute_hooks() {
    let registry = registry_for("sleep 300", "user");
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        registry.execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            Path::new("."),
            "s",
        ),
    )
    .await
    .expect("query hook dispatch must return while the background hook is still running");
}

#[cfg(unix)]
#[tokio::test]
async fn allowing_no_progress_stop_is_delivered_as_an_observation() {
    let mut config = async_config("sleep 5");
    config.timeout = Some(1);
    let registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![config],
        }],
        Some("project"),
    );
    let aggregate = registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            Path::new("."),
            "s",
        )
        .await;
    assert!(
        !aggregate.is_blocked(),
        "diagnostics cannot alter hook policy"
    );
    let diagnostic = wait_diagnostic(&registry).await;
    assert_eq!(diagnostic.outcome, "no_progress_stop");
}

#[cfg(unix)]
fn async_config(command: &str) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: command.to_owned(),
        if_condition: None,
        timeout: Some(2),
        once: None,
        r#async: Some(true),
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: vec![],
        on_failure: Some(crate::hooks::HookFailurePolicy::Allow),
        enabled: true,
    }
}
