//! Recovery through the production adapter, executor and stream collector.
#![allow(clippy::await_holding_lock)]
use archon_core::{
    agent::AgentConfig, agents::AgentRegistry, subagent::SubagentManager,
    subagent_executor::AgentSubagentExecutor,
};
use archon_llm::{
    provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo},
    streaming::StreamEvent,
};
use archon_tools::{
    subagent_dispatch_clock::within_named, subagent_executor::install_subagent_executor,
};
use archon_workflow::{
    WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess, WorkflowLlmClient,
    llm_retry::run_agent_with_transient_retry,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(super) enum Fault {
    Connection,
    Unavailable,
    Overload,
    Auth,
    Rejected,
    Partial,
    Stall,
    Context,
    Summary,
    RejectedStream,
    StreamLimited,
    PartialConnection,
}
pub(super) struct Transport {
    fault: Fault,
    calls: AtomicUsize,
    times: Mutex<Vec<tokio::time::Instant>>,
}
#[async_trait::async_trait]
impl LlmProvider for Transport {
    fn name(&self) -> &str {
        "AC-X-001"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: archon_llm::provider::ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.times.lock().unwrap().push(tokio::time::Instant::now());
        if matches!(self.fault, Fault::Unavailable) && call < 12 {
            std::future::pending::<()>().await;
        }
        if matches!(self.fault, Fault::Summary) && call == 0 {
            return Err(archon_llm::anthropic::ApiError::HttpError(
                "context window exceeded; reduce request.messages to fit context".into(),
            )
            .into());
        }
        if matches!(self.fault, Fault::Summary) && call < 13 {
            tokio::time::sleep(Duration::from_millis(1300)).await;
            return Err(LlmError::Http("connection reset".into()));
        }
        if matches!(self.fault, Fault::Connection | Fault::Overload) && call < 12 {
            tokio::time::sleep(Duration::from_millis(700)).await;
            return Err(match self.fault {
                Fault::Connection => LlmError::Http("connection reset".into()),
                _ => archon_llm::anthropic::ApiError::Overloaded {
                    retry_after_secs: None,
                    message: String::new(),
                }
                .into(),
            });
        }
        if matches!(self.fault, Fault::Context) {
            return Err(archon_llm::anthropic::ApiError::HttpError(
                "context window exceeded; reduce request.messages to fit context".into(),
            )
            .into());
        }
        if matches!(self.fault, Fault::StreamLimited) && call < 12 {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        if call == 0 {
            match self.fault {
                Fault::Auth => {
                    return Err(archon_llm::anthropic::ApiError::AuthError(
                        "request timed out; set auth.mode=AC-X-001".into(),
                    )
                    .into());
                }
                Fault::Rejected => {
                    return Err(archon_llm::anthropic::ApiError::Rejected {
                        error_type: "invalid_request_error".into(),
                        message: "request timed out; set request.shape=AC-X-001".into(),
                    }
                    .into());
                }
                _ => {}
            }
        }
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let fault = self.fault;
        tokio::spawn(async move {
            if matches!(fault, Fault::StreamLimited) && call < 12 {
                tx.send(StreamEvent::Error {
                    error_type: "rate_limit_error".into(),
                    message: "AC-X-001".into(),
                })
                .await
                .unwrap();
            } else if matches!(fault, Fault::PartialConnection) {
                tx.send(StreamEvent::TextDelta {
                    index: 0,
                    text: "AC-X-001".into(),
                })
                .await
                .unwrap();
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = tx
                    .send(StreamEvent::Error {
                        error_type: "network".into(),
                        message: "connection reset".into(),
                    })
                    .await;
            } else if matches!(fault, Fault::RejectedStream) {
                tx.send(StreamEvent::Error {
                    error_type: "invalid_request_error".into(),
                    message: "request timed out; set request.shape=AC-X-001".into(),
                })
                .await
                .unwrap();
            } else if matches!(fault, Fault::Partial) {
                tx.send(StreamEvent::TextDelta {
                    index: 0,
                    text: "AC-X-001".into(),
                })
                .await
                .unwrap();
                tx.closed().await;
            } else if matches!(fault, Fault::Stall) {
                tx.closed().await;
            } else {
                tx.send(StreamEvent::TextDelta {
                    index: 0,
                    text: "AC-X-001".into(),
                })
                .await
                .unwrap();
                tx.send(StreamEvent::MessageDelta {
                    stop_reason: Some("end_turn".into()),
                    usage: None,
                })
                .await
                .unwrap();
                tx.send(StreamEvent::MessageStop).await.unwrap();
            }
        });
        Ok(rx)
    }
}
pub(super) fn transport(fault: Fault) -> Arc<Transport> {
    Arc::new(Transport {
        fault,
        calls: AtomicUsize::new(0),
        times: Mutex::new(vec![]),
    })
}
pub(super) fn harness(
    provider: Arc<dyn LlmProvider>,
    root: &std::path::Path,
    idle: u64,
) -> Arc<dyn WorkflowLlmClient> {
    harness_with_inactivity(provider, root, idle, 30)
}

pub(super) fn harness_with_inactivity(
    provider: Arc<dyn LlmProvider>,
    root: &std::path::Path,
    idle: u64,
    inactivity: u64,
) -> Arc<dyn WorkflowLlmClient> {
    // Use the same provider retry decorator as the runtime factory.
    let provider: Arc<dyn LlmProvider> = Arc::new(archon_llm::RetryProvider::new(
        provider,
        archon_llm::RetryPolicy::default(),
    ));
    let path = root.join(".archon/agents");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("AC-X-001.md"),
        "---\nname: AC-X-001\ndescription: AC-X-001\nmodel: AC-X-001\n---\nAC-X-001\n",
    )
    .unwrap();
    let registry = AgentRegistry::load_with_user_home(root, None);
    let config = AgentConfig {
        session_id: "AC-X-001".into(),
        working_dir: root.into(),
        subagent_stream_idle_timeout_secs: idle,
        ..Default::default()
    };
    let executor = AgentSubagentExecutor::new(
        provider.clone(),
        archon_core::dispatch::ToolRegistry::new(),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(registry)),
        None,
        None,
        root.into(),
        "AC-X-001".into(),
        "AC-X-001".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new(String::new())),
        Arc::new(config),
        Arc::new(archon_llm::identity::IdentityProvider::new(
            archon_llm::identity::IdentityMode::Clean,
            "AC-X-001".into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
    crate::command::pipeline_workflow_llm::recovery_client_for_test(provider, root, inactivity)
}
pub(super) fn request() -> WorkflowAgentCall {
    WorkflowAgentCall {
        session_id: "AC-X-001".into(),
        task: "TASK-X-001".into(),
        cwd: None,
        ordinal: 0,
        attempt: 1,
        agent: WorkflowAgentSpec {
            key: "AC-X-001".into(),
            display_name: "AC-X-001".into(),
            model: "AC-X-001".into(),
            phase: 0,
            critical: true,
            parallelizable: false,
            quality_threshold: 0.0,
            tool_access: WorkflowAgentToolAccess::ReadOnly,
        },
        messages: vec![serde_json::json!({"role":"user","content":"AC-X-001"})],
        system: vec![],
        tools: vec![],
        allowed_tools: vec![],
        timeout_secs: Some(1),
        disable_auto_background: true,
        read_roots: vec![],
        write_roots: vec![],
        provider_env: None,
    }
}
async fn recovery(fault: Fault) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(fault);
    let client = harness_with_inactivity(provider.clone(), root.path(), 30, 1);
    let start = tokio::time::Instant::now();
    let retries = AtomicUsize::new(0);
    let result = within_named(
        Duration::from_secs(1),
        run_agent_with_transient_retry(&client, request(), |_| {
            retries.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        }),
    )
    .await;
    assert!(
        result.as_ref().is_ok_and(|r| r.is_ok()),
        "outage reached caller: {result:?}"
    );
    assert!(start.elapsed() >= Duration::from_secs(3));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 13);
    assert_eq!(retries.load(Ordering::SeqCst), 12);
    let times = provider.times.lock().unwrap();
    assert!(
        times
            .iter()
            .filter(|time| **time - times[0] < Duration::from_secs(60))
            .count()
            <= 11
    );
    let failed_attempt = if matches!(fault, Fault::Unavailable) {
        Duration::from_secs(30)
    } else {
        Duration::from_millis(700)
    };
    let delays = [
        500, 1000, 2000, 4000, 8000, 8000, 8000, 8000, 8000, 8000, 8000, 8000,
    ];
    for (pair, delay) in times.windows(2).zip(delays) {
        assert_eq!(
            pair[1] - pair[0],
            failed_attempt + Duration::from_millis(delay)
        );
    }
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_connection_outage_excludes_all_clocks() {
    recovery(Fault::Connection).await;
    recovery(Fault::Unavailable).await;
    stall(Fault::Stall).await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_typed_overload_recovers_through_executor() {
    recovery(Fault::Overload).await;
}
async fn permanent(fault: Fault, remedy: &str) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(fault);
    let client = harness(provider.clone(), root.path(), 30);
    let error = run_agent_with_transient_retry(&client, request(), |_| async {
        panic!("permanent fault retried")
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains(remedy), "{error}");
    assert_provider_type(&error);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_auth_type_survives_executor() {
    permanent(Fault::Auth, "set auth.mode=AC-X-001").await;
}
#[tokio::test(start_paused = true)]
async fn workflow_live_v2_rejection_type_survives_executor() {
    permanent(Fault::Rejected, "set request.shape=AC-X-001").await;
    permanent(Fault::RejectedStream, "set request.shape=AC-X-001").await;
}
async fn stall(fault: Fault) {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(fault);
    let (idle, deadline) = if matches!(fault, Fault::Stall) {
        (30, 1)
    } else {
        (1, 15)
    };
    let client = harness(provider.clone(), root.path(), idle);
    let start = tokio::time::Instant::now();
    let work = within_named(
        Duration::from_secs(deadline),
        run_agent_with_transient_retry(
            &client,
            {
                let mut r = request();
                r.timeout_secs = Some(60);
                r
            },
            |_| async { Ok(()) },
        ),
    );
    let result = tokio::select! { result=work => result, ()=tokio::time::sleep(Duration::from_secs(45)) => panic!("partial stream renewed no-progress forever") };
    assert!(
        result.is_err(),
        "expected the author deadline, got {result:?}"
    );
    assert_eq!(start.elapsed(), Duration::from_secs(deadline));
    if matches!(fault, Fault::Stall) {
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    } else {
        assert!(
            provider.calls.load(Ordering::SeqCst) >= 5,
            "partial stream never reached a second session"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_partial_stream_keeps_no_progress_accounting() {
    stall(Fault::Partial).await;
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_context_fault_survives_executor_without_outer_retry() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(Fault::Context);
    let client = harness(provider, root.path(), 30);
    let error = run_agent_with_transient_retry(&client, request(), |_| async {
        panic!("context fault retried")
    })
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reduce request.messages to fit context"),
        "{error}"
    );
    assert_provider_type(&error);
}

pub(super) fn assert_provider_type(error: &archon_workflow::WorkflowError) {
    let root: &(dyn std::error::Error + 'static) = match error {
        archon_workflow::WorkflowError::Port(source) => source.as_ref(),
        other => other,
    };
    assert!(
        std::iter::successors(Some(root), |error| error.source())
            .any(|source| source.is::<LlmError>()),
        "provider type lost: {error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_summary_provider_outage_excludes_all_clocks() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(Fault::Summary);
    let client = harness_with_inactivity(provider.clone(), root.path(), 30, 1);
    let result = within_named(
        Duration::from_secs(1),
        run_agent_with_transient_retry(&client, request(), |_| async { Ok(()) }),
    )
    .await;
    assert!(
        result.as_ref().is_ok_and(|result| result.is_ok()),
        "summary outage reached caller: {result:?}"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 14);
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_stream_rate_limit_uses_shared_recovery() {
    recovery(Fault::StreamLimited).await;
}

#[tokio::test(start_paused = true)]
async fn workflow_live_v2_partial_connection_fault_retains_elapsed_work() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let root = tempfile::tempdir().unwrap();
    let provider = transport(Fault::PartialConnection);
    let client = harness(provider.clone(), root.path(), 30);
    let start = tokio::time::Instant::now();
    let work = within_named(
        Duration::from_secs(5),
        run_agent_with_transient_retry(
            &client,
            {
                let mut call = request();
                call.timeout_secs = Some(60);
                call
            },
            |_| async { Ok(()) },
        ),
    );
    let result = tokio::select! {
        result = work => result,
        () = tokio::time::sleep(Duration::from_secs(45)) => panic!("partial connection faults erased elapsed work"),
    };
    assert!(result.is_err(), "expected author deadline, got {result:?}");
    assert!(provider.calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(start.elapsed(), Duration::from_millis(5500));
}
