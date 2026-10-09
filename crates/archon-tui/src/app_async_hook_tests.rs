use super::{App, TuiEvent};
use crate::event_loop::tui_events::handle_tui_event;
use archon_core::agent::{Agent, AgentConfig, AgentEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use std::sync::Arc;

struct ClosedStreamProvider;

#[async_trait::async_trait]
impl LlmProvider for ClosedStreamProvider {
    fn name(&self) -> &str {
        "closed-stream"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<archon_llm::provider::LlmResponse, LlmError> {
        unreachable!("test only executes hooks")
    }
}

#[tokio::test]
async fn executed_hook_diagnostic_reaches_the_tui_event_handler_without_ending_turn() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(4);
    let mut agent = Agent::new(
        Arc::new(ClosedStreamProvider),
        ToolRegistry::new(),
        AgentConfig::default(),
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let hooks = Arc::new(HookRegistry::new());
    hooks.register_matchers(
        HookEvent::PostToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "printf 'hook failed'; exit 1".into(),
                if_condition: None,
                timeout: Some(2),
                once: None,
                r#async: Some(true),
                async_rewake: None,
                status_message: None,
                headers: Default::default(),
                allowed_env_vars: vec![],
                on_failure: Some(archon_core::hooks::HookFailurePolicy::Allow),
                enabled: true,
            }],
        }],
        Some("project"),
    );
    agent.set_hook_registry(Arc::clone(&hooks));
    hooks
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            std::path::Path::new("."),
            "session",
        )
        .await;

    let timestamped = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("hook event should reach the agent channel")
        .expect("agent event channel should remain open");
    let AgentEvent::AsyncHookDiagnostic(diagnostic) = timestamped.inner else {
        panic!("expected the executed hook's diagnostic event");
    };
    let tui_event = TuiEvent::from_async_hook_diagnostic(diagnostic);

    let mut app = App::new();
    app.on_generation_started();
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    handle_tui_event(&mut app, tui_event, &input_tx).await;

    assert!(app.is_generating, "diagnostics must not end the turn");
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|line| { line.contains("async hook PostToolUse [failure] source=project") }),
        "unexpected transcript: {:?}",
        app.output.all_lines()
    );
    let stored = agent.close_async_hook_diagnostics();
    assert_eq!(stored.diagnostics.len(), 1);
}
