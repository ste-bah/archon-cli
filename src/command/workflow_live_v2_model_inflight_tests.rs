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
