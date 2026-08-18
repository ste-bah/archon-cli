#[tokio::test]
#[ignore = "Gate 5 end-to-end approval lifecycle fixture"]
async fn plan_approval_live_smoke() {
    use archon_session::plan::{PlanApprovalDecision, PlanStatus};

    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(2);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::plan_mode::ExitPlanModeTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    let config = AgentConfig {
        session_id: "plan-approval-live-smoke".into(),
        max_turns: Some(1),
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(ExitPlanCompletionProvider::two_plan_revisions()),
        registry,
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
    let preflight = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_preflight = Arc::clone(&preflight);
    agent.set_guardrail_action_id(Some("plan-approval-live-smoke".into()));
    agent.set_first_tool_action_callback(Arc::new(move |_, name, id, input| {
        observed_preflight
            .lock()
            .unwrap()
            .push((name.to_string(), id.to_string(), input.clone()));
        None
    }));
    response_tx.send("reject: missing tests".into()).await.unwrap();
    response_tx.send("approve".into()).await.unwrap();

    agent.process_message("finish the initial plan").await.unwrap();

    assert_eq!(*agent.config.permission_mode.lock().await, "plan");
    assert_eq!(agent.state.mode, AgentMode::Plan);
    let initial = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan("plan-approval-live-smoke")
        .unwrap()
        .expect("rejected plan must be persisted");
    assert_eq!(initial.title, "Initial Plan");
    assert_eq!(initial.status, PlanStatus::Abandoned);
    let rejected_ledger = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_approval_events("plan-approval-live-smoke", &initial.id)
        .unwrap();
    assert_eq!(rejected_ledger.len(), 1);
    assert_eq!(
        rejected_ledger[0].approval.decision,
        PlanApprovalDecision::Reject {
            reason: "missing tests".into()
        }
    );
    let rejected_exit_message = agent
        .state
        .messages
        .iter()
        .find(|message| {
            message["role"] == "assistant"
                && message["content"].as_array().is_some_and(|content| {
                    content.iter().any(|block| {
                        block["type"] == "tool_use" && block["id"] == "exit-plan-rejected"
                    })
                })
        })
        .expect("rejected ExitPlanMode assistant message must be retained");
    assert_eq!(
        rejected_exit_message["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|block| {
                block["type"] == "text"
                    && block["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("# Plan: Initial Plan"))
            })
            .count(),
        1,
        "rejected ExitPlanMode message must contain its plan draft exactly once"
    );
    let blocked_write = agent
        .preflight_tools(
            &[PendingToolCall {
                id: "write-after-reject".into(),
                name: "Write".into(),
                input_json: r#"{"file_path":"/tmp/blocked","content":"blocked"}"#.into(),
            }],
            AgentMode::Plan,
        )
        .await;
    assert!(blocked_write.is_empty(), "Plan-mode rejection must retain write block");

    agent.process_message("finish the revised plan").await.unwrap();

    assert_eq!(
        preflight.lock().unwrap().as_slice(),
        [
            (
                "ExitPlanMode".to_string(),
                "exit-plan-rejected".to_string(),
                serde_json::json!({})
            ),
            (
                "ExitPlanMode".to_string(),
                "exit-plan-approved".to_string(),
                serde_json::json!({})
            ),
        ],
        "both streamed ExitPlanMode calls must pass the real preflight stage"
    );
    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.inner,
                AgentEvent::ToolCallComplete { ref name, .. } if name == "ExitPlanMode"
            ))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.inner,
                AgentEvent::AskUser { kind: AskUserPromptKind::PlanApproval, .. }
            ))
            .count(),
        2
    );
    assert_eq!(*agent.config.permission_mode.lock().await, "default");
    assert_eq!(agent.state.mode, AgentMode::Normal);
    let persisted = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan("plan-approval-live-smoke")
        .unwrap()
        .expect("postprocess must atomically persist the approved revision");
    assert_eq!(persisted.title, "Revised Plan");
    assert_eq!(persisted.status, PlanStatus::Approved);
    let approved_exit_message = agent
        .state
        .messages
        .iter()
        .find(|message| {
            message["role"] == "assistant"
                && message["content"].as_array().is_some_and(|content| {
                    content.iter().any(|block| {
                        block["type"] == "tool_use" && block["id"] == "exit-plan-approved"
                    })
                })
        })
        .expect("approved ExitPlanMode assistant message must be retained");
    assert_eq!(
        approved_exit_message["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|block| {
                block["type"] == "text"
                    && block["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("# Plan: Revised Plan"))
            })
            .count(),
        1,
        "approved ExitPlanMode message must contain its plan draft exactly once"
    );
    assert_eq!(
        archon_session::plan::PlanStore::new(session_store.db())
            .unwrap()
            .load_approval_events("plan-approval-live-smoke", &persisted.id)
            .unwrap()
            .len(),
        1
    );
}
