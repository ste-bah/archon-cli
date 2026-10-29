//! Actual response headers and error mapping through the production agent path.
#![allow(clippy::await_holding_lock)]
use super::recovery_tests::{harness, request};
use archon_llm::{
    provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature},
    providers::OpenAiProvider,
    streaming::StreamEvent,
};
use archon_workflow::llm_retry::run_agent_with_transient_retry;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// Let the OS deliver HTTP bytes on real time. Only the production recovery
// waits run on virtual time; otherwise an idle Tokio runtime can advance a
// host deadline before a loopback response reaches the socket.
pub(super) struct SocketTransport(pub(super) Arc<dyn LlmProvider>);
struct ClockMode(bool);
impl ClockMode {
    fn pause(&mut self) {
        if self.0 {
            tokio::time::pause();
            self.0 = false;
        }
    }
}
impl Drop for ClockMode {
    fn drop(&mut self) {
        self.pause();
    }
}

#[async_trait::async_trait]
impl LlmProvider for SocketTransport {
    fn name(&self) -> &str {
        "AC-X-001"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        tokio::time::resume();
        let mut clock = ClockMode(true);
        let mut opening = Box::pin(self.0.stream(request));
        let opened = loop {
            tokio::select! {
                result = &mut opening => break result,
                () = tokio::time::sleep(Duration::from_millis(1)) => {
                    if archon_llm::provider_fault::pending_response().is_some_and(|(status, _)| status >= 400) {
                        clock.pause();
                        break opening.await;
                    }
                }
            }
        };
        let result = match opened {
            Ok(mut incoming) => {
                let mut events = vec![];
                while let Some(event) = incoming.recv().await {
                    events.push(event);
                }
                let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
                for event in events {
                    tx.try_send(event).unwrap();
                }
                Ok(rx)
            }
            Err(error) => Err(error),
        };
        result
    }
}

async fn endpoint(
    status: u16,
    body: &str,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    endpoint_inner(status, body, false).await
}

pub(super) async fn endpoint_inner(
    status: u16,
    body: &str,
    alternate: bool,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    endpoint_mode(status, body, alternate, false, false, false).await
}

pub(super) async fn endpoint_mode(
    status: u16,
    body: &str,
    alternate: bool,
    local: bool,
    unavailable: bool,
    stalled_body: bool,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    endpoint_fault(
        status,
        body,
        alternate,
        local,
        unavailable,
        stalled_body,
        false,
        true,
    )
    .await
}

pub(super) async fn endpoint_disconnect(
    alternate: bool,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    endpoint_fault(
        503, "AC-X-001", alternate, !alternate, true, true, true, true,
    )
    .await
}

pub(super) async fn endpoint_without_hint(
    alternate: bool,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    endpoint_fault(
        429, "AC-X-001", alternate, false, false, false, false, false,
    )
    .await
}

async fn endpoint_fault(
    status: u16,
    body: &str,
    alternate: bool,
    local: bool,
    unavailable: bool,
    stalled_body: bool,
    disconnect: bool,
    hint: bool,
) -> (
    Arc<dyn LlmProvider>,
    Arc<Mutex<Vec<tokio::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let body = body.to_string();
    let calls = Arc::new(Mutex::new(vec![]));
    let recorded = calls.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = vec![];
            loop {
                let mut byte = [0];
                if socket.read(&mut byte).await.unwrap() == 0 {
                    return;
                }
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap();
            if header.starts_with("GET ") {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await
                    .unwrap();
                continue;
            }
            let length = header
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            socket.read_exact(&mut vec![0; length]).await.unwrap();
            let count = {
                let mut calls = recorded.lock().unwrap();
                calls.push(tokio::time::Instant::now());
                calls.len()
            };
            if unavailable && count == 1 {
                if stalled_body {
                    socket.write_all(format!("HTTP/1.1 {status} AC-X-001\r\nRetry-After: 90\r\nContent-Length: 8\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                }
                if disconnect {
                    continue;
                }
                tokio::spawn(async move {
                    let _ = socket.read(&mut [0]).await;
                });
                continue;
            }
            let response = if count == 1 {
                let retry_after = if hint { "Retry-After: 90\r\n" } else { "" };
                format!(
                    "HTTP/1.1 {status} AC-X-001\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                let data = if alternate {
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"AC-X-001\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
                } else {
                    "data: {\"choices\":[{\"delta\":{\"content\":\"AC-X-001\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
                };
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
                    data.len()
                )
            };
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (
        Arc::new(SocketTransport(if local {
            Arc::new(archon_llm::providers::LocalProvider::new(
                url,
                "AC-X-001".into(),
                if stalled_body { 300 } else { 1 },
                false,
            ))
        } else if alternate {
            let auth = archon_llm::auth::AuthProvider::ApiKey(archon_llm::types::Secret::new(
                "AC-X-002".into(),
            ));
            let identity = archon_llm::identity::IdentityProvider::new(
                archon_llm::identity::IdentityMode::Clean,
                "AC-X-001".into(),
                String::new(),
                String::new(),
            );
            let client = if unavailable {
                archon_llm::anthropic::AnthropicClient::with_read_backstop(
                    auth,
                    identity,
                    Some(url),
                    if stalled_body { 300 } else { 1 },
                )
            } else {
                archon_llm::anthropic::AnthropicClient::new(auth, identity, Some(url))
            };
            Arc::new(archon_llm::providers::AnthropicProvider::new(client))
        } else {
            Arc::new(OpenAiProvider::new(
                "AC-X-001".into(),
                Some(url),
                "AC-X-001".into(),
            ))
        })),
        calls,
        server,
    )
}
async fn permanent(status: u16) {
    permanent_inner(status, false).await;
    permanent_inner(status, true).await;
}

async fn permanent_inner(status: u16, alternate: bool) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let (provider, calls, server) = endpoint_inner(
        status,
        "request timed out; set request.value=AC-X-001",
        alternate,
    )
    .await;
    let client = harness(provider, root.path(), 30);
    let result = run_agent_with_transient_retry(&client, request(), |_| async {
        panic!("permanent fault retried")
    })
    .await;
    server.abort();
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("set request.value=AC-X-001"),
        "{error}"
    );
    super::recovery_tests::assert_provider_type(&error);
    assert_eq!(calls.lock().unwrap().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_400_remedy_survives_executor() {
    permanent(400).await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_401_remedy_survives_executor() {
    permanent(401).await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_403_remedy_survives_executor() {
    permanent(403).await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_404_remedy_survives_executor() {
    permanent(404).await;
}
async fn header(status: u16) {
    header_inner(status, false).await;
    header_inner(status, true).await;
}

async fn header_inner(status: u16, alternate: bool) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let (provider, calls, server) = endpoint_inner(status, "AC-X-001", alternate).await;
    let client = harness(provider, root.path(), 30);
    let result = run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }).await;
    server.abort();
    result.unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[1] - calls[0] >= Duration::from_secs(90),
        "header wait lost: {:?}",
        calls[1] - calls[0]
    );
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_http_503_header_controls_backoff() {
    header(503).await;
    header(429).await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_status_precedes_quoted_body() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let (provider, _, server) = endpoint(502, "quoted prior response (400); AC-X-001").await;
    let client = harness(provider, root.path(), 30);
    let result = run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }).await;
    server.abort();
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_cancel_interrupts_real_provider_backoff_within_one_tick() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let (provider, calls, server) = endpoint(503, "AC-X-001").await;
    let client = harness(provider, root.path(), 30);
    let store = archon_workflow::WorkflowStore::new(root.path().join("AC-X-001"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "AC-X-001".into(),
            task: "TASK-X-001".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let ready = Arc::new(tokio::sync::Notify::new());
    let signal = ready.clone();
    let copy = store.clone();
    let id = run.id.clone();
    let cancel = tokio::spawn(async move {
        signal.notified().await;
        archon_workflow::LifecycleController::new(copy)
            .apply(&id, archon_workflow::LifecycleAction::Cancel)
            .unwrap();
        tokio::time::Instant::now()
    });
    let work = run_agent_with_transient_retry(&client, request(), |_| {
        ready.notify_one();
        async { Ok(()) }
    });
    let result =
        archon_workflow::control_race::until_run_stops(&store, &run.id, "AC-X-001", work).await;
    server.abort();
    assert!(
        matches!(
            result,
            Err(archon_workflow::WorkflowError::ControlCancelled(_))
        ),
        "{result:?}"
    );
    let cancelled_at = cancel.await.unwrap();
    assert!(cancelled_at.elapsed() <= Duration::from_secs(2));
    assert_eq!(calls.lock().unwrap().len(), 1);
}
