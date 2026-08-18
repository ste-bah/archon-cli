#[tokio::test]
async fn approved_exit_restores_permitted_bypass_mode() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: "permitted-bypass-plan-session".into(),
        allow_bypass_permissions: true,
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.mode = AgentMode::Plan;
    agent.plan_mode_state.lock().await.previous_permission_mode =
        Some(PermissionMode::BypassPermissions);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Permitted Bypass\n## Steps\n1. Restore the authorized mode"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        *agent.config.permission_mode.lock().await,
        PermissionMode::BypassPermissions.to_string()
    );
    assert_eq!(agent.state.mode, AgentMode::Normal);
}

#[tokio::test]
async fn approved_exit_preserves_operator_mode_changed_during_approval() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: "operator-downgrade-plan-session".into(),
        allow_bypass_permissions: true,
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = PermissionMode::AcceptEdits.to_string();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.mode = AgentMode::Plan;
    agent.plan_mode_state.lock().await.previous_permission_mode =
        Some(PermissionMode::BypassPermissions);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Concurrent Downgrade\n## Steps\n1. Preserve the operator mode"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        *agent.config.permission_mode.lock().await,
        PermissionMode::AcceptEdits.to_string(),
        "an operator mode change that already left Plan must win over stale restore state"
    );
    assert_eq!(agent.state.mode, AgentMode::Normal);
}

#[tokio::test]
async fn approved_exit_downgrades_unpermitted_bypass_mode() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: "unpermitted-bypass-plan-session".into(),
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.mode = AgentMode::Plan;
    agent.plan_mode_state.lock().await.previous_permission_mode =
        Some(PermissionMode::BypassPermissions);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Unpermitted Bypass\n## Steps\n1. Restore safely"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(*agent.config.permission_mode.lock().await, "default");
    assert_eq!(agent.state.mode, AgentMode::Normal);
}
