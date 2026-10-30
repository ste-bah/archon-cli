//! The retry log line's error class, captured in a binary of its own.
//!
//! tracing caches callsite interest process-wide. A scoped subscriber in the
//! shared unit-test binary races parallel tests that emit the same callsite
//! with no subscriber, which can cache `never` and empty the capture. Here the
//! capture subscriber is global, installed before the retry loop, and kept
//! until the process exits.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use archon_llm::provider::LlmError;
use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentOutcome, WorkflowAgentSpec, WorkflowAgentToolAccess,
    WorkflowLlmClient,
};
use archon_workflow::llm_retry::run_agent_with_transient_retry;
use serde_json::Value;

struct LogClient(AtomicUsize);

#[async_trait::async_trait]
impl WorkflowLlmClient for LogClient {
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(&self, _: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(WorkflowError::port(LlmError::Http(
                "Cookie: opaque; connection reset".into(),
            )))
        } else {
            Ok(WorkflowAgentOutcome::default())
        }
    }
}

struct Writer(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn request() -> WorkflowAgentCall {
    WorkflowAgentCall {
        session_id: "AC-X-001".to_string(),
        task: "TASK-X-001".to_string(),
        cwd: None,
        ordinal: 1,
        attempt: 1,
        agent: WorkflowAgentSpec {
            key: "AC-X-001".to_string(),
            display_name: "AC-X-001".to_string(),
            model: String::new(),
            phase: 1,
            critical: true,
            parallelizable: true,
            quality_threshold: 0.8,
            tool_access: WorkflowAgentToolAccess::Full,
        },
        messages: Vec::new(),
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: Some(7200),
        disable_auto_background: false,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}

#[tokio::test(start_paused = true)]
async fn llm_retry_r4_log_class_uses_raw_typed_facts() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Writer(captured.clone()))
        .finish();
    // Global for the rest of the process: no callsite can cache `never`.
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let client: Arc<dyn WorkflowLlmClient> = Arc::new(LogClient(AtomicUsize::new(0)));
    run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) })
        .await
        .unwrap();
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    // The typed fault is operational (its stored text says "connection
    // reset"), but 86fbab995's log class read the redacted Display.
    assert!(text.contains("error_class=\"transient\""), "{text}");
}
