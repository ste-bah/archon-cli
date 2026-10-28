use super::*;
use crate::llm_client_port::{WorkflowAgentSpec, WorkflowAgentToolAccess};

#[tokio::test(start_paused = true)]
async fn typed_non_retryable_provider_faults_reach_the_caller_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RejectingClient {
        calls: AtomicUsize,
        message: &'static str,
    }
    #[async_trait::async_trait]
    impl crate::llm_client_port::WorkflowLlmClient for RejectingClient {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            unreachable!()
        }

        async fn run_agent(
            &self,
            _: WorkflowAgentCall,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                call, 0,
                "non-retryable fault invoked provider more than once"
            );
            Err(WorkflowError::port(std::io::Error::other(self.message)))
        }
    }

    let cases = [
        ("HTTP 400: invalid request; request timed out", "HTTP 400"),
        (
            "HTTP 401: authentication failed; request timed out",
            "HTTP 401",
        ),
        ("HTTP 403: permission denied; request timed out", "HTTP 403"),
        ("HTTP 404: model not found; request timed out", "HTTP 404"),
        (
            "context window exceeded: request timed out; reduce input tokens",
            "context window exceeded",
        ),
    ];

    for (message, remedy) in cases {
        let fake = Arc::new(RejectingClient {
            calls: AtomicUsize::new(0),
            message,
        });
        let client: Arc<dyn WorkflowLlmClient> = fake.clone();
        let error = run_agent_with_transient_retry(&client, request(false), |_| async { Ok(()) })
            .await
            .expect_err("typed provider rejection was retried or swallowed");
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "{message}");
        assert!(error.to_string().contains(remedy), "{error}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_workflow_agent_recovers_with_a_callback_for_every_transient_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecoveringClient(AtomicUsize, std::sync::Mutex<Vec<tokio::time::Instant>>);
    #[async_trait::async_trait]
    impl crate::llm_client_port::WorkflowLlmClient for RecoveringClient {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            unreachable!()
        }

        async fn run_agent(
            &self,
            _: WorkflowAgentCall,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            self.1.lock().unwrap().push(tokio::time::Instant::now());
            if self.0.fetch_add(1, Ordering::SeqCst) < 12 {
                Err(crate::error::WorkflowError::port(std::io::Error::other(
                    "connection reset",
                )))
            } else {
                Ok(crate::llm_client_port::WorkflowAgentOutcome {
                    content: "recovered".into(),
                    ..Default::default()
                })
            }
        }
    }

    let fake = Arc::new(RecoveringClient(
        AtomicUsize::new(0),
        std::sync::Mutex::new(Vec::new()),
    ));
    let client: Arc<dyn WorkflowLlmClient> = fake.clone();
    let mut retries = Vec::new();
    let outcome = run_agent_with_transient_retry(&client, request(false), |attempt| {
        retries.push(attempt);
        async { Ok(()) }
    })
    .await
    .expect("a transient provider outage should recover");
    assert_eq!(outcome.content, "recovered");
    assert_eq!(retries, (1..=12).collect::<Vec<_>>());
    let attempts = fake.1.lock().unwrap();
    for (index, pair) in attempts.windows(2).enumerate() {
        assert_eq!(
            pair[1] - pair[0],
            Duration::from_millis(500 * (1u64 << index).min(16))
        );
    }
    assert_eq!(
        attempts
            .iter()
            .filter(|time| **time - attempts[0] < Duration::from_secs(60))
            .count(),
        11,
        "an unavailable endpoint must not cause a busy loop"
    );
}

#[test]
fn workflow_foreground_subagent_timeout_is_not_transient_retry() {
    let request = request(true);

    assert!(!transient_live_agent_error_for_request(
        &request,
        "subagent timed out after 7200s"
    ));
    assert!(transient_live_agent_error_for_request(
        &request,
        "provider request timed out"
    ));
}

/// A transport stall marker must be handled by the provider retry path.
#[test]
fn a_transport_stall_enters_the_backed_off_retry_path() {
    let text = format!(
        "subagent failed: {} subagent stream retry exhausted: no answer (last: stream idle timeout); error sending request",
        crate::error::TRANSPORT_STALL_MARKER
    );
    assert!(transient_live_agent_error(&text));
    assert!(transient_live_agent_error_for_request(
        &request(false),
        &text
    ));
    let wrapped = WorkflowError::port(std::io::Error::other(text.clone()));
    assert!(crate::error::is_transport_stall(&wrapped));
    assert!(!crate::error::is_transport_stall(
        &WorkflowError::StageFailed("error sending request".into())
    ));
}

/// An inactivity cut says "timeout" too, and is the host's decision exactly
/// like the wall clock: never a provider blink to re-ask.
#[test]
fn workflow_inactivity_cut_is_not_transient_retry() {
    let text = format!(
        "agent transport failed: {} no model output, tool call or tool result for 1800s",
        crate::error::INACTIVITY_TIMEOUT_MARKER
    );
    assert!(
        transient_live_agent_error(&text),
        "the bare classifier matches"
    );
    assert!(!transient_live_agent_error_for_request(
        &request(true),
        &text
    ));
}

#[test]
fn auto_background_timeout_keeps_provider_retry_behavior() {
    let request = request(false);

    assert!(transient_live_agent_error_for_request(
        &request,
        "subagent timed out after 30s"
    ));
}

pub(super) fn request(disable_auto_background: bool) -> WorkflowAgentCall {
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
        disable_auto_background,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}

/// A port client that cannot restore a completed invocation refuses to
/// continue one instead of answering with a fresh call (#241).
#[tokio::test]
async fn a_port_client_that_cannot_restore_refuses_to_continue() {
    struct Stateless;
    #[async_trait::async_trait]
    impl crate::llm_client_port::WorkflowLlmClient for Stateless {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            panic!("a continuation was answered with a fresh call")
        }
    }
    use crate::llm_client_port::WorkflowLlmClient;
    let error = Stateless
        .continue_agent(request(true))
        .await
        .expect_err("a stateless client continued");
    assert!(
        error.continuation_refusal().is_some(),
        "not a typed refusal: {error}"
    );
}
