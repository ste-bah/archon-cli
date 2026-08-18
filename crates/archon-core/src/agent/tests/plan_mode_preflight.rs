#[tokio::test]
async fn plan_mode_preflight_writes_to_the_session_audit() {
    let temp = tempfile::tempdir().unwrap();
    let mut agent = test_agent();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.config.session_id = "preflight-session".to_string();
    let tool = PendingToolCall {
        id: "tool-1".to_string(),
        name: "Write".to_string(),
        input_json: r#"{"file_path":"/tmp/preflight"}"#.to_string(),
    };

    assert!(
        !agent
            .plan_mode_allows_tool(
                &tool,
                &serde_json::json!({"file_path": "/tmp/preflight"}),
                AgentMode::Plan,
            )
            .await
    );

    let audit_path = crate::plan_file::plan_audit_path(temp.path(), "preflight-session").unwrap();
    let audit = std::fs::read_to_string(audit_path).unwrap();
    assert!(audit.contains("Write (intercepted in Plan Mode)"));
    assert!(audit.contains("/tmp/preflight"));
}

#[tokio::test]
async fn plan_mode_preflight_audits_before_cognitive_and_permission_gates() {
    let temp = tempfile::tempdir().unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    let mut config = AgentConfig::default();
    config.working_dir = temp.path().to_path_buf();
    config.session_id = "preflight-gate-order".into();
    *config.permission_mode.lock().await = "default".into();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.classify_cognitive_situation("hello");
    let tool = PendingToolCall {
        id: "write-before-other-gates".into(),
        name: "Write".into(),
        input_json: r#"{"file_path":"/tmp/preflight","content":"blocked"}"#.into(),
    };

    assert!(
        agent
            .preflight_single_tool(&tool, AgentMode::Plan)
            .await
            .is_none(),
    );

    let audit_path = crate::plan_file::plan_audit_path(temp.path(), "preflight-gate-order")
        .unwrap();
    let audit = std::fs::read_to_string(audit_path).unwrap();
    assert_eq!(audit.matches("Write (intercepted in Plan Mode)").count(), 1);
    assert!(audit.contains("/tmp/preflight"));

    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.inner,
                AgentEvent::ToolCallComplete { ref name, ref id, .. }
                    if name == "Write" && id == "write-before-other-gates"
            ))
            .count(),
        1,
        "Plan Mode rejection must emit exactly one completion event"
    );
}

#[tokio::test]
async fn plan_mode_preflight_rejects_unsafe_session_id_without_creating_audit() {
    let temp = tempfile::tempdir().unwrap();
    let mut agent = test_agent();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.config.session_id = "../../escape".to_string();
    let tool = PendingToolCall {
        id: "tool-unsafe".to_string(),
        name: "Write".to_string(),
        input_json: r#"{"file_path":"/tmp/preflight"}"#.to_string(),
    };

    assert!(
        !agent
            .plan_mode_allows_tool(
                &tool,
                &serde_json::json!({"file_path": "/tmp/preflight"}),
                AgentMode::Plan,
            )
            .await
    );
    assert!(!temp.path().join("escape.md").exists());
    assert!(!temp.path().join(".archon/plan-audit").exists());
}
