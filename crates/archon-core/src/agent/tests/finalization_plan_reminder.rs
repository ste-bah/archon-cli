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
            captured_system: Arc::clone(&captured_system),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.process_message("prepare a plan").await.unwrap();

    let request = captured_system.lock().unwrap().pop().unwrap();
    let reminder = request
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.contains("<system-reminder>"))
        .expect("plan request should contain a workflow reminder");
    assert!(reminder.contains("read-only tools"));
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
            captured_system: Arc::clone(&captured_system),
        }),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.process_message("answer normally").await.unwrap();

    let request = captured_system.lock().unwrap().pop().unwrap();
    assert!(
        !request
            .iter()
            .filter_map(|block| block["text"].as_str())
            .any(|text| text.contains("ExitPlanMode"))
    );
}
