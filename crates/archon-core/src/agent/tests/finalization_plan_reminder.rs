#[tokio::test]
async fn plan_mode_uses_configured_plan_model_and_origin() {
    let captured_requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        model: "session".into(),
        ..AgentConfig::default()
    };
    config.context = toml::from_str("plan_model = \"planner\"").unwrap();
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            captured_requests: Arc::clone(&captured_requests),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );

    agent.process_message("prepare a plan").await.unwrap();

    let request = captured_requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.model, "planner");
    assert_eq!(request.request_origin.as_deref(), Some("plan_mode"));
}

#[tokio::test]
async fn approved_execution_reverts_to_session_model() {
    let captured_requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        model: "session".into(),
        ..AgentConfig::default()
    };
    config.context = toml::from_str("plan_model = \"planner\"").unwrap();
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let override_model = Arc::clone(&config.model_override);
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            captured_requests: Arc::clone(&captured_requests),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );

    agent.process_message("prepare a plan").await.unwrap();
    *agent.config.permission_mode.lock().await = PermissionMode::Auto.to_string();
    agent.process_message("execute the approved plan").await.unwrap();

    // Scoped so the guard is released before the await below. A `MutexGuard`
    // is not `Send`, so holding one across an await point stops the future
    // being spawned at all — harmless here only because nothing spawns it.
    {
        let requests = captured_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].model, "planner");
        assert_eq!(requests[0].request_origin.as_deref(), Some("plan_mode"));
        assert_eq!(requests[1].model, "session");
        assert_eq!(requests[1].request_origin.as_deref(), Some("main_session"));
    }
    assert!(override_model.lock().await.is_empty());
}

#[tokio::test]
async fn captured_plan_request_contains_plan_workflow_reminder() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig::default();
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls,
            captured_requests: Arc::clone(&captured_system),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.process_message("prepare a plan").await.unwrap();

    let request = captured_system.lock().unwrap().pop().unwrap().system;
    let reminder = request
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.contains("<system-reminder>"))
        .expect("plan request should contain a workflow reminder");
    assert!(reminder.contains("working-tree mutations"));
    assert!(reminder.contains("TaskCreate"));
    assert!(reminder.contains("TaskUpdate"));
    assert!(reminder.contains("Agent"));
    assert!(reminder.contains("goal"));
    assert!(reminder.contains("files"));
    assert!(reminder.contains("ordered dependency steps"));
    assert!(reminder.contains("verification shape"));
    assert!(reminder.contains("Persist"));
    assert!(reminder.contains("ExitPlanMode"));
}

#[tokio::test]
async fn captured_normal_request_omits_plan_workflow_reminder() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls,
            captured_requests: Arc::clone(&captured_system),
        }),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.process_message("answer normally").await.unwrap();

    let request = captured_system.lock().unwrap().pop().unwrap().system;
    assert!(
        !request
            .iter()
            .filter_map(|block| block["text"].as_str())
            .any(|text| text.contains("ExitPlanMode"))
    );
}
