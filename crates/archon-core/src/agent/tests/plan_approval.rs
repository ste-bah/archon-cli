use std::sync::Arc;

#[test]
fn approval_prompt_renders_stored_title_and_ordered_steps() {
    use archon_session::plan::{PlanDocument, PlanStep, PlanStepStatus};

    let mut plan = PlanDocument::new("approval-render", "Stored title");
    plan.steps = vec![
        PlanStep {
            number: 2,
            description: "second stored step".into(),
            affected_files: Vec::new(),
            status: PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        },
        PlanStep {
            number: 1,
            description: "first stored step".into(),
            affected_files: Vec::new(),
            status: PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        },
    ];

    let prompt = crate::agent::plan_approval::render_plan_approval(&plan);

    assert!(prompt.contains("Stored title"));
    assert!(prompt.contains("approve"));
    assert!(prompt.contains("approve-edits"));
    assert!(prompt.contains("edit"));
    assert!(prompt.contains("reject: <reason>"));
    assert!(prompt.find("first stored step") < prompt.find("second stored step"));
}

#[test]
fn approval_response_requires_a_nonempty_rejection_reason() {
    use archon_session::plan::PlanApprovalDecision;

    assert_eq!(
        crate::agent::plan_approval::parse_plan_approval_response("approve"),
        Ok(PlanApprovalDecision::Approve)
    );
    assert_eq!(
        crate::agent::plan_approval::parse_plan_approval_response("approve-edits"),
        Ok(PlanApprovalDecision::ApproveAcceptEdits)
    );
    assert_eq!(
        crate::agent::plan_approval::parse_plan_approval_response("edit"),
        Ok(PlanApprovalDecision::Edit)
    );
    assert!(crate::agent::plan_approval::parse_plan_approval_response("reject: ").is_err());
}

#[tokio::test]
async fn exit_plan_waits_for_approval_before_restoring_default() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(1);
    let config = AgentConfig {
        session_id: "approval-prompt-session".into(),
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(response_rx)));
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Stored Approval Plan\n## Steps\n1. Keep Plan Mode until approval"}]
    }));
    response_tx.send("approve".into()).await.unwrap();

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(*agent.config.permission_mode.lock().await, "default");
    assert_eq!(agent.state.mode, AgentMode::Normal);
    let prompt =
        std::iter::from_fn(|| event_rx.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::AskUser { question, .. } => Some(question),
            _ => None,
        });
    let prompt = prompt.expect("approval prompt must be emitted before approval can restore mode");
    assert!(prompt.contains("Stored Approval Plan"));
    assert!(prompt.contains("Keep Plan Mode until approval"));
    let persisted = agent
        .plan_store
        .as_ref()
        .unwrap()
        .load_latest_plan("approval-prompt-session")
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, archon_session::plan::PlanStatus::Approved);
    assert_eq!(persisted.steps.len(), 1);
    let task_id = persisted.steps[0].task_id.as_ref().unwrap();
    assert!(
        archon_tools::task_manager::TASK_MANAGER
            .get_task(task_id)
            .is_some_and(|task| task.metadata.is_some())
    );
    assert_eq!(
        agent
            .plan_store
            .as_ref()
            .unwrap()
            .load_plan_tasks("approval-prompt-session")
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn rejected_exit_returns_reason_and_keeps_write_blocked() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(1);
    let config = AgentConfig {
        session_id: "rejected-plan-session".into(),
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(response_rx)));
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Rejected Plan\n## Steps\n1. Do not write"}]
    }));
    response_tx
        .send("reject: missing tests".into())
        .await
        .unwrap();

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(result.is_error);
    assert_eq!(result.content, "Error: Plan rejected: missing tests");
    assert_eq!(*agent.config.permission_mode.lock().await, "plan");
    assert_eq!(agent.state.mode, AgentMode::Plan);
    assert_eq!(
        agent.plan_mode_state.lock().await.previous_permission_mode,
        Some(PermissionMode::Default)
    );
}

#[tokio::test]
async fn approve_edits_restores_accept_edits() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(1);
    let config = AgentConfig::default();
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(response_rx)));
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Edit Plan\n## Steps\n1. Permit edits"}]
    }));
    response_tx.send("approve-edits".into()).await.unwrap();

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(*agent.config.permission_mode.lock().await, "acceptEdits");
    assert_eq!(agent.state.mode, AgentMode::Normal);
}

#[tokio::test]
async fn slash_entry_restores_recorded_default_not_auto() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig::default();
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Slash Plan\n## Steps\n1. Restore default"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(*agent.config.permission_mode.lock().await, "default");
    assert_ne!(*agent.config.permission_mode.lock().await, "auto");
}

#[tokio::test]
async fn invalid_interactive_approval_reprompts_without_terminal_persistence() {
    use archon_session::plan::PlanStatus;

    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(3);
    let config = AgentConfig {
        session_id: "reprompt-plan-session".into(),
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(response_rx)));
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Reprompt Plan\n## Steps\n1. Await valid approval"}]
    }));
    response_tx.send("reject: ".into()).await.unwrap();
    response_tx.send("unknown".into()).await.unwrap();
    response_tx.send("approve".into()).await.unwrap();

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::AskUser { .. }))
            .count(),
        3,
        "initial prompt plus one reprompt per invalid response"
    );
    let loaded = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan("reprompt-plan-session")
        .unwrap()
        .expect("approved plan");
    assert_eq!(loaded.status, PlanStatus::Approved);
    assert_eq!(
        archon_session::plan::PlanStore::new(session_store.db())
            .unwrap()
            .load_approval_events("reprompt-plan-session", &loaded.id)
            .unwrap()
            .len(),
        1,
        "invalid responses must not create terminal ledger records"
    );
}

#[tokio::test]
async fn noninteractive_exit_auto_approves_and_persists_ledger_event() {
    use archon_session::plan::{PlanApprovalDecision, PlanApprovalSource, PlanStatus};

    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: "noninteractive-plan-session".into(),
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Durable Plan\n## Steps\n1. Persist approval"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    let loaded = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan("noninteractive-plan-session")
        .unwrap()
        .expect("approved plan persisted");
    assert_eq!(loaded.status, PlanStatus::Approved);
    assert_eq!(
        loaded.approval.as_ref().unwrap().source,
        PlanApprovalSource::NonInteractive
    );
    let ledger = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_approval_events("noninteractive-plan-session", &loaded.id)
        .unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(
        ledger[0].approval.source,
        PlanApprovalSource::NonInteractive
    );
    assert_eq!(ledger[0].approval.decision, PlanApprovalDecision::Approve);
}

#[tokio::test]
async fn noninteractive_reject_policy_fails_closed_and_retains_plan_mode() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig::default();
    config.context.noninteractive_plan_approval = "reject".into();
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
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "# Plan: Rejected by Policy\n## Steps\n1. Stay blocked"}]
    }));

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("rejected by policy"));
    assert_eq!(*agent.config.permission_mode.lock().await, "plan");
    assert_eq!(agent.state.mode, AgentMode::Plan);
}

#[path = "plan_approval_materialization.rs"]
mod materialization;

#[test]
fn plan_store_attachment_rejects_corrupt_durable_task_state() {
    use archon_session::plan::{PersistedPlanTask, PlanDocument, PlanStatus, PlanStep, PlanStepStatus};

    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = archon_session::plan::PlanStore::new(&db).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: format!("corrupt-rehydrate-{}", uuid::Uuid::new_v4()),
        ..AgentConfig::default()
    };
    let session_id = config.session_id.clone();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let plan_id = "corrupt-plan";
    let task_id = format!("corrupt-{}", uuid::Uuid::new_v4());
    let mut plan = PlanDocument::new(plan_id, "Corrupt plan");
    plan.status = PlanStatus::Approved;
    plan.steps = vec![PlanStep {
        number: 1,
        description: "corrupt durable task".into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
        task_id: Some(task_id.clone()),
    }];
    store.save_plan(&session_id, &plan).unwrap();
    store
        .save_plan_task_fixture(
            &session_id,
            &PersistedPlanTask {
                task_id,
                plan_id: plan_id.into(),
                plan_step: 1,
                description: "corrupt durable task".into(),
                status: "Corrupt".into(),
                blocked_by: vec![],
                required_evidence: vec![],
                updated_at: "2026-08-15T00:00:00Z".into(),
            },
        )
        .unwrap();

    let authority = store
        .bootstrap_approval_authority_for_test(&session_id)
        .unwrap();
    let error = agent.set_plan_store(store, authority).unwrap_err();
    assert!(error.contains("unknown persisted task status: Corrupt"));
    assert!(agent.plan_store.is_none());
}
