use super::*;
use crate::llm_client_port::WorkflowAgentOutcome;
use archon_llm::provider::LlmError;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RejectingClient {
    calls: AtomicUsize,
    fault: Mutex<Option<LlmError>>,
}

#[derive(Debug, thiserror::Error)]
#[error("request failed: {0}")]
struct WrappedFault(#[source] LlmError);

#[async_trait::async_trait]
impl WorkflowLlmClient for RejectingClient {
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
        assert_eq!(
            self.calls.fetch_add(1, Ordering::SeqCst),
            0,
            "fault retried"
        );
        // The production port preserves the provider's error source chain.
        Err(WorkflowError::port(WrappedFault(
            self.fault.lock().unwrap().take().unwrap(),
        )))
    }
}

async fn assert_once(fault: LlmError) {
    let remedy = fault.to_string();
    let fake = Arc::new(RejectingClient {
        calls: AtomicUsize::new(0),
        fault: Mutex::new(Some(fault)),
    });
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    let error = run_agent_with_transient_retry(&client, super::tests::request(false), |_| async {
        panic!("non-retryable fault scheduled a retry")
    })
    .await
    .unwrap_err();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    let WorkflowError::Port(source) = error else {
        panic!("typed provider fault was flattened: {error}")
    };
    let mut current = Some(source.as_ref() as &(dyn std::error::Error + 'static));
    let mut found = false;
    while let Some(error) = current {
        if let Some(fault) = error.downcast_ref::<LlmError>() {
            assert_eq!(fault.to_string(), remedy, "remedy changed");
            found = true;
        }
        current = error.source();
    }
    assert!(found, "provider error lost its type and remedy");
}

fn status_fault(status: u16, remedy: &str) -> LlmError {
    LlmError::Server {
        status,
        message: format!("request timed out; {remedy}"),
        pipeline_retry_after_secs: None,
    }
}

#[tokio::test(start_paused = true)]
async fn typed_400_reaches_the_caller_once_with_its_remedy() {
    assert_once(status_fault(400, "set request.max_tokens=1")).await;
}

#[tokio::test(start_paused = true)]
async fn typed_401_reaches_the_caller_once_with_its_remedy() {
    // Authentication mappings can omit the numeric HTTP status entirely.
    assert_once(LlmError::Auth(
        "request timed out; set credential=AC-X-001".into(),
    ))
    .await;
}

#[tokio::test(start_paused = true)]
async fn typed_403_reaches_the_caller_once_with_its_remedy() {
    assert_once(status_fault(403, "set credential.scope=AC-X-001")).await;
}

#[tokio::test(start_paused = true)]
async fn typed_404_reaches_the_caller_once_with_its_remedy() {
    assert_once(status_fault(404, "set request.model=AC-X-001")).await;
}

#[tokio::test(start_paused = true)]
async fn typed_context_overflow_reaches_the_caller_once_with_its_remedy() {
    assert_once(LlmError::ContextWindowExceeded {
        provider_message: "request timed out; reduce request.messages to fit context".into(),
        provider: None,
        model: None,
    })
    .await;
}
