use super::*;
use archon_workflow::WorkflowAgentCall;

use crate::command::tui_workflow_ui_sink::{default_workflow_ui_sink_parts, try_fill_one};

#[tokio::test]
async fn closed_tui_prevents_cached_script_host_call_reuse() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let script = r#"
async function workflow(w) {
  await w.checkpoint("cached-call");
}
"#;
    let (initial_ui_sink, _initial_tui_rx) = default_workflow_ui_sink();
    let initial_client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        initial_ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let initial_runner = WorkflowV2ScriptRunner::new(
        "cached call".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        initial_client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    initial_runner.run(script).await.expect("initial summary");
    let mut checkpoint = v2_store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");
    checkpoint.remove_completed_call("cached-call");
    v2_store
        .save_checkpoint(&checkpoint)
        .expect("reset checkpoint");

    let (closed_ui_sink, closed_tui_rx) = default_workflow_ui_sink();
    drop(closed_tui_rx);
    let closed_client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        closed_ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let closed_runner = WorkflowV2ScriptRunner::new(
        "cached call".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        closed_client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let error = closed_runner
        .run(script)
        .await
        .expect_err("closed TUI must reject cached reuse");
    let checkpoint = v2_store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");

    assert!(matches!(error, WorkflowError::NotificationDelivery(_)));
    assert!(
        !checkpoint
            .completed_call_ids
            .iter()
            .any(|id| id == "cached-call"),
        "cached call was marked complete after status delivery failed"
    );
}

#[tokio::test]
async fn closed_tui_prevents_script_host_provider_result_persistence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let provider = Arc::new(CompletionBlockedScriptLlm::default());
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        provider.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "blocked provider completion".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );
    let handle = tokio::spawn(async move {
        runner
            .run(
                r#"
async function workflow(w) {
  await w.agent("blocked-agent", { role: "analysis", task: "Return accepted result" });
}
"#,
            )
            .await
    });

    provider.started.notified().await;
    drop(tui_rx);
    provider.release.notify_one();
    let error = handle
        .await
        .expect("script join")
        .expect_err("closed TUI must reject provider completion");

    assert!(matches!(error, WorkflowError::NotificationDelivery(_)));
    assert!(
        v2_store
            .load_call_record("blocked-agent")
            .expect("call record lookup")
            .is_none(),
        "provider result persisted after completion status delivery failed"
    );
    assert!(
        v2_store
            .load_checkpoint()
            .expect("checkpoint lookup")
            .is_none(),
        "checkpoint persisted after completion status delivery failed"
    );
}

#[tokio::test]
async fn closed_tui_during_provider_repair_prevents_script_host_persistence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let provider = Arc::new(RepairBlockedScriptLlm::default());
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        provider.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "blocked provider repair".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );
    let handle = tokio::spawn(async move {
        runner
            .run(
                r#"
async function workflow(w) {
  await w.agent("repair-agent", { role: "analysis", task: "Return accepted result" });
}
"#,
            )
            .await
    });

    provider.repair_started.notified().await;
    drop(tui_rx);
    provider.release_repair.notify_one();
    let error = handle
        .await
        .expect("script join")
        .expect_err("closed TUI must reject provider repair");

    assert!(matches!(error, WorkflowError::NotificationDelivery(_)));
    assert!(
        v2_store
            .load_call_record("repair-agent")
            .expect("call record lookup")
            .is_none()
    );
    assert!(
        v2_store
            .load_checkpoint()
            .expect("checkpoint lookup")
            .is_none()
    );
}

#[tokio::test]
async fn full_tui_during_provider_retry_waits_before_retrying() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let provider = Arc::new(TransientBlockedScriptLlm::default());
    let (ui_sink, fill_tx, mut tui_rx) = default_workflow_ui_sink_parts();
    let client = LiveV2AgentClient::new(
        provider.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "blocked provider retry".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );
    let handle = tokio::spawn(async move {
        runner
            .run(
                r#"
async function workflow(w) {
  await w.agent("retry-agent", { role: "analysis", task: "Return accepted result" });
}
"#,
            )
            .await
    });

    provider.started.notified().await;
    let _running = tui_rx.recv().await.expect("initial running activity");
    while try_fill_one(&fill_tx) {}
    provider.release.notify_one();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let _freed = tui_rx.recv().await.expect("free retry-status capacity");
    provider.second_started.notified().await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    drop(tui_rx);
    let error = handle
        .await
        .expect("script join")
        .expect_err("closed TUI must reject failed status");

    assert!(matches!(error, WorkflowError::NotificationDelivery(_)));
    assert!(
        v2_store
            .load_call_record("retry-agent")
            .expect("call record lookup")
            .is_none()
    );
    assert!(
        v2_store
            .load_checkpoint()
            .expect("checkpoint lookup")
            .is_none()
    );
}

#[derive(Default)]
pub(super) struct TransientBlockedScriptLlm {
    pub(super) calls: AtomicUsize,
    pub(super) started: tokio::sync::Notify,
    pub(super) second_started: tokio::sync::Notify,
    pub(super) release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for TransientBlockedScriptLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            self.started.notify_one();
            self.release.notified().await;
        } else {
            self.second_started.notify_one();
        }
        Err(archon_workflow::WorkflowError::port("429 rate limit"))
    }
}

#[derive(Default)]
pub(super) struct CompletionBlockedScriptLlm {
    pub(super) started: tokio::sync::Notify,
    pub(super) release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CompletionBlockedScriptLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.started.notify_one();
        self.release.notified().await;
        let mut result = WorkflowV2Result::accepted("accepted");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "provider completed",
        ));
        Ok(WorkflowAgentOutcome {
            content: serde_json::to_string(&result).expect("result json"),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[derive(Default)]
pub(super) struct RepairBlockedScriptLlm {
    pub(super) calls: AtomicUsize,
    pub(super) first_finished: tokio::sync::Notify,
    pub(super) repair_started: tokio::sync::Notify,
    pub(super) release_repair: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for RepairBlockedScriptLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            self.first_finished.notify_one();
            return Ok(WorkflowAgentOutcome {
                content: "malformed result".to_string(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            });
        }
        self.repair_started.notify_one();
        self.release_repair.notified().await;
        Ok(WorkflowAgentOutcome {
            content: "unused repair result".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}
