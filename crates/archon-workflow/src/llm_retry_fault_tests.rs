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
        message: format!("request timed out; {remedy}").into(),
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

#[tokio::test(start_paused = true)]
async fn llm_retry_r4_send_uses_raw_typed_facts() {
    struct Recovering(AtomicUsize);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for Recovering {
        async fn send_message(
            &self,
            _: Vec<Value>,
            _: Vec<Value>,
            _: Vec<Value>,
            _: &str,
        ) -> WorkflowResult<WorkflowAgentOutcome> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(WorkflowError::port(LlmError::Http(
                    "Cookie: opaque; connection reset".into(),
                )))
            } else {
                Ok(WorkflowAgentOutcome::default())
            }
        }
    }
    let fake = Arc::new(Recovering(AtomicUsize::new(0)));
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    let mut retries = Vec::new();
    // 86fbab995's send-message policy read the redacted Display, where the
    // cookie line hides "connection reset": it did not retry.
    let result =
        send_message_with_transient_retry(&client, vec![], vec![], vec![], "test", |attempt| {
            retries.push(attempt);
            async { Ok(()) }
        })
        .await;
    assert!(result.is_err());
    assert!(retries.is_empty());
    assert_eq!(fake.0.load(Ordering::SeqCst), 1);
}

// llm_retry_r4_log_class_uses_raw_typed_facts lives in its own test binary,
// tests/llm_retry_log_class.rs: tracing caches callsite interest per process.

struct OnceThenOk(AtomicUsize, Mutex<Option<WorkflowError>>);

#[async_trait::async_trait]
impl WorkflowLlmClient for OnceThenOk {
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.next()
    }
    async fn run_agent(&self, _: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        self.next()
    }
}

impl OnceThenOk {
    fn new(error: WorkflowError) -> Arc<Self> {
        Arc::new(Self(AtomicUsize::new(0), Mutex::new(Some(error))))
    }
    fn next(&self) -> WorkflowResult<WorkflowAgentOutcome> {
        self.0.fetch_add(1, Ordering::SeqCst);
        match self.1.lock().unwrap().take() {
            Some(error) => Err(error),
            None => Ok(WorkflowAgentOutcome::default()),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn llm_retry_r6_agent_retry_reads_visible_wrapper_text_like_main() {
    // 86fbab995: is_operational() || the visible display's transient text.
    // The wrapper's own "request failed" text is visible, so main retried.
    let fake = OnceThenOk::new(WorkflowError::port(WrappedFault(LlmError::Http(
        "unclassified failure".into(),
    ))));
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    run_agent_with_transient_retry(&client, super::tests::request(false), |_| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(fake.0.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn llm_retry_r6_send_reads_visible_wrapper_text_like_main() {
    #[derive(Debug, thiserror::Error)]
    #[error("HTTP 503")]
    struct Visible(#[source] LlmError);
    let fake = OnceThenOk::new(WorkflowError::port(Visible(LlmError::Http(
        "ordinary failure".into(),
    ))));
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    send_message_with_transient_retry(&client, vec![], vec![], vec![], "test", |_| async {
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(fake.0.load(Ordering::SeqCst), 2);
}
