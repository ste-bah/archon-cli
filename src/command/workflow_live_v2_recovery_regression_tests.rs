//! Permanent payloads and timeout metadata through the real HTTP executor.
#![allow(clippy::await_holding_lock)]
use super::recovery_http_tests::endpoint_inner;
use super::recovery_tests::{harness, request};
use archon_workflow::llm_retry::run_agent_with_transient_retry;
use std::time::Duration;

async fn rejection(status: u16, code: &str) {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let body = serde_json::json!({"error": {"code": code, "type": code,
            "message": "request timed out; set request.value=AC-X-001"}})
        .to_string();
        let (provider, calls, server) = endpoint_inner(status, &body, alternate).await;
        let client = harness(provider, root.path(), 30);
        let result = tokio::time::timeout(
            Duration::from_secs(300),
            run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }),
        )
        .await;
        server.abort();
        let error = result
            .expect("rejection retried forever")
            .expect_err("permanent payload was retried");
        assert!(
            error.to_string().contains("set request.value=AC-X-001"),
            "{error}"
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_quota_payload_is_permanent_through_executor() {
    rejection(429, "insufficient_quota").await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_invalid_model_payload_is_permanent_through_executor() {
    rejection(500, "model_not_found").await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_408_recovers_through_executor() {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let (provider, calls, server) = endpoint_inner(408, "AC-X-001", alternate).await;
        let client = harness(provider, root.path(), 30);
        let result = tokio::time::timeout(
            Duration::from_secs(300),
            run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }),
        )
        .await;
        server.abort();
        assert!(
            result.is_ok_and(|result| result.is_ok()),
            "408 did not recover"
        );
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_504_uses_header_instead_of_display_text() {
    for alternate in [false, true] {
        let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
        let root = tempfile::tempdir().unwrap();
        let (provider, calls, server) =
            endpoint_inner(504, "retry after 86400; AC-X-001", alternate).await;
        let client = harness(provider, root.path(), 30);
        let result = tokio::time::timeout(
            Duration::from_secs(300),
            run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }),
        )
        .await;
        server.abort();
        assert!(
            result.is_ok_and(|result| result.is_ok()),
            "display text overrode header"
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let delay = calls[1] - calls[0];
        assert!(
            delay >= Duration::from_secs(90) && delay < Duration::from_secs(91),
            "{delay:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_validation_continuation_heals_http_outage() {
    use super::recovery_http_tests::SocketTransport;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let recorded = calls.clone();
    let mut accepted = archon_workflow::WorkflowV2Result::accepted("AC-X-001");
    accepted
        .evidence
        .push(archon_workflow::WorkflowV2Evidence::new(
            archon_workflow::WorkflowV2EvidenceKind::Inspection,
            "AC-X-001",
        ));
    let accepted = serde_json::to_string(&accepted).unwrap();
    Mock::given(method("POST")).respond_with(move |_: &wiremock::Request| {
        let call = recorded.fetch_add(1, Ordering::SeqCst);
        if (1..=5).contains(&call) {
            ResponseTemplate::new(503).insert_header("Retry-After", "90").set_body_string("AC-X-001")
        } else {
            let text = if call == 0 { "AC-X-001" } else { &accepted };
            let chunk = serde_json::json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]});
            ResponseTemplate::new(200).set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
        }
    }).mount(&server).await;
    let root = tempfile::tempdir().unwrap();
    let provider = Arc::new(SocketTransport(Arc::new(
        archon_llm::providers::OpenAiProvider::new(
            "AC-X-001".into(),
            Some(server.uri()),
            "AC-X-001".into(),
        ),
    )));
    let port = harness(provider, root.path(), 30);
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = super::LiveV2AgentClient::new(
        port,
        sink,
        vec!["AC-X-001".into()],
        "AC-X-001".into(),
        None,
        Some(10),
    );
    let mut request = super::tests::request(archon_workflow::WorkflowV2HostMethod::Agent, None);
    request.role = "AC-X-001".into();
    request.task = "TASK-X-001".into();
    request.target_files.clear();
    request.repository_root = Some(root.path().to_string_lossy().into_owned());
    let result = tokio::time::timeout(
        Duration::from_secs(1000),
        super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
            &archon_workflow::WorkflowV2AgentAdapter::new(),
            &client,
            &request,
            None,
        ),
    )
    .await;
    assert!(
        result.as_ref().is_ok_and(|result| result.is_ok()),
        "repair outage escaped: {result:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    let received = server.received_requests().await.unwrap();
    let last: serde_json::Value = serde_json::from_slice(&received.last().unwrap().body).unwrap();
    let first_repair: serde_json::Value = serde_json::from_slice(&received[1].body).unwrap();
    assert_eq!(
        last["messages"], first_repair["messages"],
        "recovery changed the pending repair history"
    );
    assert!(
        last["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "assistant"
                && message.to_string().contains("AC-X-001")),
        "continuation lost the initial answer: {last}"
    );
}

async fn local_rejection(status: u16, code: &str) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let body = serde_json::json!({"error":{"code":code,
        "message":"request timed out; set request.value=AC-X-001"}})
    .to_string();
    let (provider, calls, server) =
        super::recovery_http_tests::endpoint_mode(status, &body, false, true, false, false).await;
    let client = harness(provider, root.path(), 30);
    let result = tokio::time::timeout(
        Duration::from_secs(300),
        run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }),
    )
    .await;
    server.abort();
    let error = result
        .expect("rejection retried forever")
        .expect_err("permanent payload was retried");
    assert!(
        error.to_string().contains("set request.value=AC-X-001"),
        "{error}"
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_local_quota_reaches_executor_once() {
    local_rejection(429, "insufficient_quota").await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_local_invalid_model_reaches_executor_once() {
    local_rejection(500, "model_not_found").await;
}
