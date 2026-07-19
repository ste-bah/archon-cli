    #[tokio::test]
    async fn awaited_host_call_time_does_not_trip_js_watchdog() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(SlowAcceptedLlm {
                delay: WORKFLOW_JS_WATCHDOG + Duration::from_millis(25),
            }),
            tui_tx,
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        let runner = WorkflowV2ScriptRunner::new(
            "slow awaited host call".to_string(),
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

        let summary = runner
            .run(
                r#"
async function workflow(w) {
  const slow = await w.agent("slow-agent", { role: "analysis", task: "Return accepted result after a slow host await" });
  const normalized = [slow.summary].map((value) => String(value).trim()).filter(Boolean);
  await w.checkpoint("after-slow-agent", { inputs: normalized, task: "Prove JS resumed after slow host await" });
}
"#,
            )
            .await
            .expect("script summary");

        assert_eq!(summary.status, WorkflowV2Status::Accepted);
        assert_eq!(summary.executed, 2);
        assert!(
            v2_store
                .load_call_record("slow-agent")
                .expect("slow agent lookup")
                .is_some()
        );
        assert!(
            v2_store
                .load_call_record("after-slow-agent")
                .expect("checkpoint lookup")
                .is_some()
        );
    }

    #[tokio::test]
    async fn workflow_js_error_returns_failed_summary_for_state_sync() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        let runner = WorkflowV2ScriptRunner::new(
            "raw js crash".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        );

        let summary = runner
            .run(
                r#"
async function workflow(w) {
  await w.checkpoint("before-crash", { task: "record a call before JS crashes" });
  throw new Error("boom after checkpoint");
}
"#,
            )
            .await
            .expect("failed summary");

        assert_eq!(summary.status, WorkflowV2Status::Failed);
        assert_eq!(summary.failed_call.as_deref(), Some("workflow.js"));
        assert_eq!(summary.executed, 1);
        super::super::workflow_live_v2_state::sync_v2_summary_to_run(
            &workflow_store,
            &run.id,
            &summary.calls,
            &v2_store,
            summary.status,
        )
        .expect("sync failed summary");
        let run_state = workflow_store.load_state(&run.id).expect("run state");
        assert_eq!(run_state.status, RunStatus::Failed);
        let events = std::fs::read_to_string(workflow_store.run_dir(&run.id).join("events.jsonl"))
            .expect("events");
        assert!(events.contains("\"event\":\"script_stopped\""));
        assert!(events.contains("\"event\":\"terminal_status\""));
    }

    fn test_spec() -> WorkflowSpec {
        WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "script-stop-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 4,
            max_agents: 16,
            provider_tiers: BTreeMap::new(),
            stages: Vec::new(),
            artifact_policy: Default::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }

    fn test_runtime(spec: &WorkflowSpec) -> WorkflowV2ScriptRuntime {
        WorkflowV2ScriptRuntime {
            target_repository_root: spec.target_repository_root.clone(),
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        }
    }

    fn task_universe() -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: vec!["/tmp/tasks".to_string()],
            tasks: vec![
                super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                    canonical_task_id: "TASK-TDL-001".to_string(),
                    aliases: vec!["T001".to_string()],
                    source_path: "/tmp/tasks/TASK-TDL-001.md".to_string(),
                    dependency_ids: Vec::new(),
                    title: None,
                    artifact_requirements: Vec::new(),
                    ..Default::default()
                },
                super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                    canonical_task_id: "TASK-TDL-010".to_string(),
                    aliases: vec!["T010".to_string()],
                    source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
                    dependency_ids: vec!["TASK-TDL-001".to_string()],
                    title: None,
                    artifact_requirements: Vec::new(),
                    ..Default::default()
                },
            ],
        }
    }

    struct PanicLlm;

    #[async_trait::async_trait]
    impl LlmClient for PanicLlm {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse> {
            panic!("local-host workflow script test must not call the LLM")
        }
    }

    struct AlwaysInvalidLlm {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LlmClient for AlwaysInvalidLlm {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                content: "not workflow v2 result json".to_string(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            })
        }
    }

    struct SlowAcceptedLlm {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl LlmClient for SlowAcceptedLlm {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse> {
            tokio::time::sleep(self.delay).await;
            let mut result = WorkflowV2Result::accepted("slow accepted");
            result.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "slow host call completed",
            ));
            Ok(LlmResponse {
                content: serde_json::to_string(&result).expect("result json"),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            })
        }
    }
