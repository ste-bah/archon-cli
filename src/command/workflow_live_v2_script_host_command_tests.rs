//! Persisted HostCommand and trusted raw-outcome script-host tests.

use super::*;

struct FakeHostCommandExecutor {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for FakeHostCommandExecutor
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: format!("{} passed", request.command_id),
            stderr: String::new(),
            stdout_bytes: 20,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!("passed"),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id: format!("host-command:{}:fixed", request.command_id),
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-08-27T00:00:00Z".into(),
            }),
            subjects: Vec::new(),
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

#[tokio::test]
async fn host_command_uses_persisted_call_record_and_checkpoint_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "host command persistence".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(executor.clone());

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  const result = await w.hostCommand("task-set-lint", { stdin: null });
  if (result.exitCode !== 0 || !result.stdout.includes("passed")) {
    throw new Error("typed host command result missing");
  }
  return result;
}
"#,
        )
        .await
        .expect("host command script");

    let call_id = "host-command:task-set-lint:fixed";
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(summary.executed, 1);
    let record = v2_store
        .load_call_record(call_id)
        .expect("record lookup")
        .expect("persisted host command record");
    assert_eq!(record.call.method, WorkflowV2HostMethod::HostCommand);
    assert_eq!(record.status, WorkflowV2Status::Accepted);
    assert_eq!(record.result.data["exitCode"], 0);
    assert!(
        v2_store
            .load_call_record("hostCommand#1")
            .expect("transport id lookup")
            .is_none(),
        "transport correlation id must not become reuse identity"
    );
    let checkpoint = v2_store
        .load_checkpoint()
        .expect("checkpoint lookup")
        .expect("checkpoint");
    assert!(checkpoint.completed_call_ids.contains(&call_id.to_string()));
}

#[tokio::test]
async fn host_command_is_reused_without_second_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let script = r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#;

    for _ in 0..2 {
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            ui_sink.clone(),
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        WorkflowV2ScriptRunner::new(
            "host command reuse".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_host_command_executor(executor.clone())
        .run(script)
        .await
        .expect("host command run");
    }

    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}

struct RawOutcomeLlm {
    calls: AtomicUsize,
    requests: std::sync::Mutex<Vec<archon_workflow::WorkflowAgentCall>>,
    content: &'static str,
    stop_reason: &'static str,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for RawOutcomeLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed raw outcome must use run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request);
        Ok(WorkflowAgentOutcome {
            content: self.content.to_string(),
            tool_uses: Vec::new(),
            tokens_in: 3,
            tokens_out: 5,
            stop_reason: Some(self.stop_reason.to_string()),
        })
    }
}

#[tokio::test]
async fn trusted_raw_outcome_bypasses_structured_parse_and_repair() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let llm = Arc::new(RawOutcomeLlm {
        calls: AtomicUsize::new(0),
        requests: std::sync::Mutex::new(Vec::new()),
        content: "opaque candidate bytes, not WorkflowV2Result JSON",
        stop_reason: "max_tokens",
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(1_500),
    )
    .with_fixed_raw_tool_policy(vec![
        "Read".into(),
        "Grep".into(),
        "Glob".into(),
        "CartographerScan".into(),
    ]);
    let runner = WorkflowV2ScriptRunner::new(
        "raw author".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id,
        true,
        None,
        None,
    )
    .with_raw_outcomes(true);

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  const authored = await w.agent("acceptance-author-1", {
    task: "Author candidate bytes",
    tier: "planner",
    resultMode: "rawOutcome"
  });
  if (authored.content !== "opaque candidate bytes, not WorkflowV2Result JSON") throw new Error("content changed");
  if (authored.stopReason !== "max_tokens") throw new Error("stop reason lost");
  return authored;
}
"#,
        )
        .await
        .expect("raw outcome run");

    assert_eq!(summary.executed, 1);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    let requests = llm.requests.lock().unwrap();
    assert_eq!(
        requests[0].allowed_tools,
        [
            "__ARCHON_EXACT_TOOLS__",
            "Read",
            "Grep",
            "Glob",
            "CartographerScan"
        ]
    );
    assert_eq!(requests[0].timeout_secs, Some(1_500));
}

#[tokio::test]
async fn untrusted_script_raw_outcome_refuses_before_provider_dispatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let llm = Arc::new(RawOutcomeLlm {
        calls: AtomicUsize::new(0),
        requests: std::sync::Mutex::new(Vec::new()),
        content: "must not dispatch",
        stop_reason: "end_turn",
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(1_500),
    );
    let runner = WorkflowV2ScriptRunner::new(
        "untrusted raw author".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id,
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"async function workflow(w) { return await w.agent("raw", { task: "author", resultMode: "rawOutcome" }); }"#,
        )
        .await
        .expect("policy failure is a typed call result");

    assert_eq!(summary.status, WorkflowV2Status::Failed);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
}

struct RejectReuseExecutor {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for RejectReuseExecutor
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        FakeHostCommandExecutor {
            calls: AtomicUsize::new(0),
        }
        .execute(request)
        .await
    }
}

#[tokio::test]
async fn host_command_generic_cache_match_still_requires_current_receipt_postcondition() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let executor = Arc::new(RejectReuseExecutor {
        calls: AtomicUsize::new(0),
    });
    let script = r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#;

    for _ in 0..2 {
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            ui_sink.clone(),
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        WorkflowV2ScriptRunner::new(
            "four-way reuse".into(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_host_command_executor(executor.clone())
        .run(script)
        .await
        .unwrap();
    }

    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
}
