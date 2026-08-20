#[tokio::test]
async fn preflight_observer_receives_first_allowed_tool_with_final_input() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        permission_mode: Arc::new(Mutex::new("bypassPermissions".to_string())),
        // This is about the permission gate, not the freshness gate. The
        // fixture writes to paths nothing has read, which read_before_edit
        // refuses by design (#193 Phase A).
        filesystem: crate::config::FilesystemConfig {
            read_before_edit: crate::config::ReadBeforeEdit::Off,
        },
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    hooks.register_callback(
        crate::hooks::HookEvent::PreToolUse,
        crate::hooks::HookCallbackEntry {
            name: "rewrite-write-path".to_string(),
            callback: Arc::new(|_| crate::hooks::HookResult {
                updated_input: Some(serde_json::json!({
                    "file_path": "/after-hook",
                    "content": "updated"
                })),
                ..Default::default()
            }),
            authority: crate::hooks::SourceAuthority::Policy,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(hooks);
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_for_callback = Arc::clone(&observed);
    agent.set_guardrail_action_id(Some("action-1".to_string()));
    agent.set_turn_requirement_reminder(Some("old requirement".into()));
    agent.set_first_tool_action_callback(Arc::new(move |action_id, name, id, input| {
        observed_for_callback.lock().unwrap().push((
            action_id.to_string(),
            name.to_string(),
            id.to_string(),
            input.clone(),
        ));
        Some("reclassified requirement".into())
    }));
    let pending = [
        PendingToolCall {
            id: "tool-1".to_string(),
            name: "Write".to_string(),
            input_json: r#"{"file_path":"/before-1","content":"before"}"#.to_string(),
        },
        PendingToolCall {
            id: "tool-2".to_string(),
            name: "Write".to_string(),
            input_json: r#"{"file_path":"/before-2","content":"before"}"#.to_string(),
        },
    ];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

    assert_eq!(allowed.len(), 2);
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        [(
            "action-1".to_string(),
            "Write".to_string(),
            "tool-1".to_string(),
            serde_json::json!({"file_path": "/after-hook", "content": "updated"}),
        )]
    );
    assert_eq!(
        agent.turn_requirement_reminder.as_deref(),
        Some("reclassified requirement")
    );
}
