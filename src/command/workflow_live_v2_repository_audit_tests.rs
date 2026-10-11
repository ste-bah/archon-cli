use super::*;

#[tokio::test]
async fn repository_audit_dispatch_uses_selected_timeout_and_read_only_tools() {
    struct Capture(Mutex<Option<WorkflowAgentCall>>);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for Capture {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
            unreachable!()
        }
        async fn run_agent(
            &self,
            call: WorkflowAgentCall,
        ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
            *self.0.lock().unwrap() = Some(call);
            Ok(WorkflowAgentOutcome {
                content: accepted_with_items(),
                tool_uses: vec![],
                tokens_in: 0,
                tokens_out: 0,
                stop_reason: Some("end_turn".into()),
            })
        }
    }
    use archon_workflow::WorkflowAgentDispatch;
    let capture = Arc::new(Capture(Mutex::new(None)));
    struct Progress(Mutex<String>);
    #[async_trait::async_trait]
    impl archon_workflow::ui_sink_port::WorkflowUiSink for Progress {
        async fn emit(
            &self,
            event: archon_workflow::WorkflowUiEvent,
        ) -> archon_workflow::ui_sink_port::WorkflowUiResult {
            if let archon_workflow::WorkflowUiEvent::Text(text) = event {
                self.0.lock().unwrap().push_str(&text);
            }
            Ok(())
        }
    }
    let progress = Arc::new(Progress(Mutex::new(String::new())));
    let ui = progress.clone();
    let client = LiveV2AgentClient::new(capture.clone(), ui, vec![], "run".into(), None, Some(17));
    let mut execution = declaring_call("audit-timeout", None);
    execution
        .call
        .options
        .extra
        .insert("audit_timeout_secs".into(), json!(7200));
    super::workflow_live_v2_script::AuditDispatch(client.for_audit())
        .run_call(
            "audit",
            None,
            &execution,
            &WorkflowV2AgentAdapter::new(),
            None,
            None,
        )
        .await
        .unwrap();
    let call = capture.0.lock().unwrap().take().unwrap();
    assert_eq!(call.timeout_secs, Some(7200));
    let text = progress.0.lock().unwrap();
    assert!(
        text.contains("Repository audit waiting") && text.contains("7200"),
        "audit wait and allowance not visible: {text}"
    );
    assert!(
        !call
            .allowed_tools
            .iter()
            .any(|t| matches!(t.as_str(), "Bash" | "Write" | "Edit" | "Agent"))
    );
}

#[tokio::test]
async fn repository_audit_direct_implementation_requires_sealed_dispatch() {
    use archon_workflow::repository_audit::{
        budget::{AuditPolicy, Limit},
        runtime::AuditRuntime,
    };
    struct InspectRoot(Mutex<Vec<std::path::PathBuf>>);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for InspectRoot {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
            unreachable!()
        }
        async fn run_agent(
            &self,
            call: WorkflowAgentCall,
        ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
            if let Some(root) = call.cwd {
                self.0.lock().unwrap().push(root);
            }
            Ok(WorkflowAgentOutcome {
                content: blocked_without_items(),
                tool_uses: vec![],
                tokens_in: 0,
                tokens_out: 0,
                stop_reason: Some("end_turn".into()),
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let store = WorkflowStore::project(temp.path().join("project"));
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "direct-write".into(),
        task: "deliver".into(),
        target_repository_root: Some(repo.display().to_string()),
        max_agents: 1,
        max_parallelism: 1,
        stages: vec![],
        permissions: Default::default(),
        learning_hooks: vec![],
    };
    let run = store.create_run(spec).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let audit = AuditRuntime::initialize(
        store.clone(),
        run.id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    let llm = Arc::new(InspectRoot(Mutex::new(vec![])));
    let (ui, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui,
        vec![],
        run.id.clone(),
        Some(repo.display().to_string()),
        None,
    )
    .with_audit(audit.clone());
    let mut execution = declaring_call("direct-write", None);
    execution.call.method = WorkflowV2HostMethod::Implementation;
    execution.call.write_mode = Some(archon_workflow::WorkflowV2WriteMode::Serial);
    execution.call.options.target_files = vec!["new.txt".into()];
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: Some(repo.display().to_string()),
        ..Default::default()
    };
    execute_v2_live_call(
        "deliver",
        &runtime,
        execution,
        WorkflowV2AgentAdapter::new(),
        &client,
        &v2,
        &store,
        &run.id,
        true,
        None,
        None,
        false,
        store.load_state(&run.id).unwrap().generation,
    )
    .await
    .unwrap();
    let roots = llm.0.lock().unwrap();
    assert!(!roots.is_empty());
    assert!(
        roots.iter().all(|root| root.join(".git").is_file()),
        "direct writer was sent to canonical repository: {roots:?}"
    );
    assert!(audit.state().unwrap().declared_paths.contains("new.txt"));
}
