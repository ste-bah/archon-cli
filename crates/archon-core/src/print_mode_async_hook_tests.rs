use super::*;
use crate::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use std::sync::{Arc, Mutex};

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
        unreachable!("test only exercises streaming")
    }
}

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture() -> (SharedWriter, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    (SharedWriter::new(Capture(Arc::clone(&bytes))), bytes)
}

async fn run_with_diagnostic(output_format: OutputFormat) -> (Vec<u8>, Vec<u8>) {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(16);
    let test_event_tx = event_tx.clone();
    let mut agent = Agent::new(
        Arc::new(ClosedStreamProvider),
        crate::dispatch::ToolRegistry::new(),
        crate::agent::AgentConfig::default(),
        event_tx,
        Arc::new(std::sync::RwLock::new(crate::agents::AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let hooks = Arc::new(HookRegistry::new());
    hooks.register_matchers(
        HookEvent::BeforeAgentRun,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "printf 'hook diagnostic'; exit 1".into(),
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
    agent.set_hook_registry(Arc::clone(&hooks));
    hooks
        .execute_hooks(
            HookEvent::BeforeAgentRun,
            serde_json::json!({"hook_event":"BeforeAgentRun"}),
            std::path::Path::new("."),
            "session",
        )
        .await;
    let observed = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("the executed hook should report into the agent event channel")
        .expect("agent event channel should remain open");
    test_event_tx.send(observed).await.unwrap();
    drop(test_event_tx);
    let (stdout, stdout_bytes) = capture();
    let (stderr, stderr_bytes) = capture();
    let config = PrintModeConfig {
        query: "test query".into(),
        output_format,
        input_format: InputFormat::Text,
        max_turns: None,
        max_budget_usd: None,
        no_session_persistence: true,
        json_schema: None,
    };
    let _ = run_print_mode_with_writers(
        config,
        &ArchonConfig::default(),
        &mut agent,
        event_rx,
        stdout,
        stderr,
    )
    .await;
    let stdout = stdout_bytes.lock().unwrap().clone();
    let stderr = stderr_bytes.lock().unwrap().clone();
    (stdout, stderr)
}

#[tokio::test]
async fn print_mode_run_end_writes_hook_diagnostics_to_stderr() {
    let (_, stderr) = run_with_diagnostic(OutputFormat::Text).await;
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(
        stderr.contains("[async hook:BeforeAgentRun:failure source=project]"),
        "run-end stderr omitted hook outcome: {stderr}"
    );
}

#[tokio::test]
async fn print_mode_json_run_end_includes_hook_diagnostics() {
    let (stdout, _) = run_with_diagnostic(OutputFormat::Json).await;
    let output = String::from_utf8(stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
    assert_eq!(
        result["async_hook_diagnostics"][0]["event"],
        "BeforeAgentRun"
    );
    assert_eq!(result["async_hook_diagnostics"][0]["source"], "project");
}
