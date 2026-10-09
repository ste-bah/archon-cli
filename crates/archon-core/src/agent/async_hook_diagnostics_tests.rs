use super::*;
use crate::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};

#[tokio::test]
async fn hook_execution_reaches_agent_event_channel_and_remains_in_session_store() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);
    let mut agent = Agent::new(
        Arc::new(super::super::tests::MockLlmProvider),
        crate::dispatch::ToolRegistry::new(),
        crate::agent::AgentConfig::default(),
        event_tx,
        Arc::new(std::sync::RwLock::new(crate::agents::AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let registry = Arc::new(HookRegistry::new());
    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "printf failure >&2; exit 1".into(),
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
        Some("project"),
    );
    agent.set_hook_registry(Arc::clone(&registry));

    registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            std::path::Path::new("."),
            "session",
        )
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("hook diagnostic should reach the agent event channel")
        .expect("agent event channel should remain open");
    let crate::agent::AgentEvent::AsyncHookDiagnostic(diagnostic) = event.inner else {
        panic!("expected async hook diagnostic event");
    };
    assert_eq!(diagnostic.outcome, "failure");
    assert_eq!(diagnostic.source.as_deref(), Some("project"));
    assert!(!diagnostic.message.is_empty());

    let stored = agent.close_async_hook_diagnostics();
    assert_eq!(stored.diagnostics.len(), 1);
    assert_eq!(stored.diagnostics[0].event, "PostToolUse");
    assert_eq!(stored.diagnostics[0].source.as_deref(), Some("project"));
}

#[tokio::test]
async fn full_agent_event_channel_keeps_diagnostic_for_session_end_drain() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);
    event_tx
        .try_send(crate::agent::TimestampedEvent {
            sent_at: std::time::Instant::now(),
            inner: crate::agent::AgentEvent::TextDelta("occupies channel".into()),
        })
        .unwrap();
    let mut agent = Agent::new(
        Arc::new(super::super::tests::MockLlmProvider),
        crate::dispatch::ToolRegistry::new(),
        crate::agent::AgentConfig::default(),
        event_tx,
        Arc::new(std::sync::RwLock::new(crate::agents::AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let registry = Arc::new(HookRegistry::new());
    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "printf full-channel; exit 1".into(),
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
        Some("project"),
    );
    agent.set_hook_registry(Arc::clone(&registry));
    registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            std::path::Path::new("."),
            "session",
        )
        .await;

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let batch = agent.drain_async_hook_diagnostics();
            if !batch.diagnostics.is_empty() {
                assert_eq!(batch.diagnostics[0].outcome, "failure");
                assert_eq!(batch.diagnostics[0].source.as_deref(), Some("project"));
                // Put the diagnostic back through a fresh hook execution so
                // the close-and-drain path below is the one that retains it.
                registry
                    .execute_hooks(
                        HookEvent::PostToolUse,
                        serde_json::json!({}),
                        std::path::Path::new("."),
                        "session",
                    )
                    .await;
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background hook should finish");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let occupied = event_rx.try_recv().expect("seed event remains queued");
    assert!(matches!(
        occupied.inner,
        crate::agent::AgentEvent::TextDelta(_)
    ));
    let retained = agent.close_async_hook_diagnostics();
    assert_eq!(retained.diagnostics.len(), 1);
    assert_eq!(retained.diagnostics[0].event, "PostToolUse");
    assert_eq!(retained.dropped, 0);
}
