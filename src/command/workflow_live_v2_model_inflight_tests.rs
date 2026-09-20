use super::*;

struct InflightInspectingLlm {
    store: WorkflowStore,
    run_id: String,
    log_path: std::path::PathBuf,
    called: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for InflightInspectingLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("fixed raw outcome uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        let root = self.store.run_dir(&self.run_id);
        let prompt = root.join("prompts/acceptance-author-1.json");
        assert!(prompt.is_file(), "raw author prompt must precede dispatch");
        let evidence: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("agent-outputs/acceptance-author-1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(evidence["status"], "running");
        assert!(
            evidence["transcript_directory"]
                .as_str()
                .unwrap()
                .contains(&self.run_id)
        );

        let v2 = WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2"));
        let record = v2
            .load_call_record("acceptance-author-1")?
            .expect("running call is durable before provider dispatch");
        assert_eq!(record.status, WorkflowV2Status::Running);
        assert_eq!(record.attempt, 1);
        let events = std::fs::read_to_string(self.store.events_path(&self.run_id)).unwrap();
        assert!(events.contains("author_attempt_started"), "{events}");
        let log = std::fs::read_to_string(&self.log_path).unwrap();
        assert!(log.contains("phase=acceptance"), "{log}");
        assert!(log.contains("attempt=1"), "{log}");

        Ok(WorkflowAgentOutcome {
            content: "candidate".into(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".into()),
        })
    }
}

#[tokio::test]
async fn fixed_author_inflight_state_event_and_log_precede_provider_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let log_path = temp.path().join("tasks/.decompose.log");
    super::workflow_live_v2_script_fixed_progress_tests::seed_fixed_progress_state(
        &store, &run.id, &log_path,
    );
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let client = LiveV2AgentClient::new(
        Arc::new(InflightInspectingLlm {
            store: store.clone(),
            run_id: run.id.clone(),
            log_path,
            called: Arc::clone(&called),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    )
    .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Glob".into()]);

    WorkflowV2ScriptRunner::new(
        "fixed inflight ordering".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2,
        store,
        run.id,
        true,
        None,
        None,
    )
    .with_raw_outcomes(true)
    .run(
        r#"async function workflow(w) {
          return await w.agent("acceptance-author-1", {
            task: "return candidate",
            tier: "planner",
            resultMode: "rawOutcome"
          });
        }"#,
    )
    .await
    .unwrap();
    assert!(called.load(std::sync::atomic::Ordering::SeqCst));
}

struct PendingInflightLlm {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

struct DropSignal(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for PendingInflightLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("fixed raw outcome uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let _drop_signal = DropSignal(Arc::clone(&self.dropped));
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn fixed_author_pause_drops_inflight_provider_and_preserves_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let log_path = temp.path().join("tasks/.decompose.log");
    super::workflow_live_v2_script_fixed_progress_tests::seed_fixed_progress_state(
        &store, &run.id, &log_path,
    );
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PendingInflightLlm {
            started: Arc::clone(&started),
            dropped: Arc::clone(&dropped),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    )
    .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Glob".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "fixed pause ordering".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2.clone(),
        store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    )
    .with_raw_outcomes(true);
    let handle = tokio::spawn(async move {
        runner
            .run(
                r#"async function workflow(w) {
                  return await w.agent("acceptance-author-1", {
                    task: "return candidate",
                    tier: "planner",
                    resultMode: "rawOutcome"
                  });
                }"#,
            )
            .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("fixed author provider must start before pause");
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(7), handle)
        .await
        .expect("fixed author pause must stop within one control interval")
        .unwrap()
        .unwrap_err();

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(&run.id)
                .join("agent-outputs/acceptance-author-1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["status"], "interrupted");

    let events = std::fs::read_to_string(store.events_path(&run.id))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        events.iter().any(|event| {
            event["kind"] == "author_attempt_interrupted"
                && event["detail"]["call_id"] == "acceptance-author-1"
        }),
        "{events:#?}"
    );
    let record = v2.load_call_record("acceptance-author-1").unwrap().unwrap();
    assert_eq!(record.attempt, 1);
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
}

#[test]
fn stale_generation_cannot_persist_accepted_fixed_call_or_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let log_path = temp.path().join("tasks/.decompose.log");
    super::workflow_live_v2_script_fixed_progress_tests::seed_fixed_progress_state(
        &store, &run.id, &log_path,
    );
    let call = WorkflowV2HostCall {
        id: "acceptance-author-1".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    let mut running = WorkflowV2Result::default();
    running.status = WorkflowV2Status::Running;
    running.summary = "fixed decomposition call in flight".into();
    let running_record = WorkflowV2CallRecord::new(
        v2.run_id(),
        call.clone(),
        1,
        "input".into(),
        running,
        Vec::new(),
    );
    v2.save_call_record(&running_record).unwrap();
    let state_before = std::fs::read(
        store
            .run_dir(&run.id)
            .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH),
    )
    .unwrap();
    let record_before = std::fs::read(v2.result_path(&call.id)).unwrap();
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let accepted = WorkflowV2CallRecord::new(
        v2.run_id(),
        call,
        1,
        "input".into(),
        WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: "stale accepted result".into(),
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );

    let error = crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::persist_generation_owned_call(
        &store,
        &run.id,
        &v2,
        &accepted,
        crate::command::workflow_decompose_state::FixedCallProjectionKind::Executed,
        Some(run.generation),
    )
    .unwrap_err();

    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(v2.result_path(&accepted.call.id)).unwrap(),
        record_before
    );
    assert_eq!(
        std::fs::read(
            store
                .run_dir(&run.id)
                .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH),
        )
        .unwrap(),
        state_before
    );
    assert!(v2.load_checkpoint().unwrap().is_none());

    let execution = include_str!("workflow_live_v2_script_host_exec.rs");
    let state = include_str!("workflow_live_v2_script_host_state.rs");
    assert!(
        execution.contains("FixedCallProjectionKind::Executed,\n            execution_generation,"),
        "{execution}"
    );
    assert!(
        state.contains("FixedCallProjectionKind::Started,\n            generation,"),
        "{state}"
    );
    assert!(
        state.contains("FixedCallProjectionKind::Reused,\n            generation,"),
        "{state}"
    );
}
