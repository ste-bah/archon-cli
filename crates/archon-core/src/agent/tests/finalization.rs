struct GuardrailCompletionProvider {
    calls: Arc<AtomicUsize>,
    captured_requests: Arc<std::sync::Mutex<Vec<LlmRequest>>>,
}

#[async_trait::async_trait]
impl LlmProvider for GuardrailCompletionProvider {
    fn name(&self) -> &str {
        "guardrail-completion"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.captured_requests.lock().unwrap().push(request);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(3);
        tx.send(StreamEvent::ThinkingDelta {
            index: 0,
            thinking: "draft thinking".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::TextDelta {
            index: 0,
            text: "final answer".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

include!("finalization_plan_reminder.rs");

#[tokio::test]
async fn blocked_finalization_retries_once_without_turn_complete() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls: Arc::clone(&calls),
            captured_requests: Arc::clone(&captured_system),
        }),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let reasoning_outputs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_reasoning_outputs = Arc::clone(&reasoning_outputs);
    agent.set_record_reasoning_turn_callback(Arc::new(move |payload| {
        captured_reasoning_outputs
            .lock()
            .unwrap()
            .push(payload.assistant_text);
    }));
    agent.set_guardrail_action_id(Some("blocked-action".into()));
    agent.set_turn_requirement_reminder(Some("RunTests".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Blocked {
        repair_prompt: "Run the required tests before finalizing.".into(),
    }));

    let error = agent
        .process_message("implement feature")
        .await
        .unwrap_err();

    assert!(matches!(error, AgentLoopError::FinalizationBlocked(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(captured_system.lock().unwrap()[0].system.iter().any(|block| {
        block["text"]
            .as_str()
            .is_some_and(|text| text.contains("RunTests"))
    }));
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::TurnComplete { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::TextDelta(_)))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::TransientThinkingDelta(ref thinking) if thinking == "draft thinking"))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::DiscardThinkingPreview))
            .count(),
        2
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::CommitThinkingPreview))
    );
    let lifecycle = events
        .iter()
        .filter_map(|event| match event.inner {
            AgentEvent::TransientThinkingDelta(_) => Some("preview"),
            AgentEvent::DiscardThinkingPreview => Some("discard"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lifecycle, ["preview", "discard", "preview", "discard"]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::ThinkingDelta(_)))
    );
    assert!(reasoning_outputs.lock().unwrap().is_empty());
    assert!(
        agent
            .conversation_state()
            .messages
            .iter()
            .all(|message| message["role"] != "assistant")
    );
}

#[tokio::test]
async fn blocked_trivial_finalization_enters_bounded_repair_loop() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls: Arc::clone(&calls),
            captured_requests: captured_system,
        }),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.set_guardrail_action_id(Some("trivial-action".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Blocked {
        repair_prompt: "Verification is required before finalizing.".into(),
    }));

    let error = agent.process_message("hello").await.unwrap_err();

    assert!(matches!(error, AgentLoopError::FinalizationBlocked(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::TurnComplete { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::TextDelta(_)))
    );
    assert!(
        agent
            .conversation_state()
            .messages
            .iter()
            .all(|message| message["role"] != "assistant")
    );
}

struct ExitPlanCompletionProvider {
    streams: std::sync::Mutex<std::collections::VecDeque<(&'static str, &'static str)>>,
}

impl Default for ExitPlanCompletionProvider {
    fn default() -> Self {
        Self {
            streams: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }
}

impl ExitPlanCompletionProvider {
    fn two_plan_revisions() -> Self {
        Self {
            streams: std::sync::Mutex::new(std::collections::VecDeque::from([
                (
                    "exit-plan-rejected",
                    "# Plan: Initial Plan\n## Steps\n1. Missing tests",
                ),
                (
                    "exit-plan-approved",
                    "# Plan: Revised Plan\n## Steps\n1. Add tests\n2. Execute safely",
                ),
            ])),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for ExitPlanCompletionProvider {
    fn name(&self) -> &str {
        "exit-plan-completion"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tool_use_id, plan) = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((
                "exit-plan-tool",
                "# Plan: Guarded Plan\n## Steps\n1. Persist this step",
            ));
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(StreamEvent::TextDelta {
            index: 0,
            text: plan.into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentBlockStart {
            index: 1,
            block_type: archon_llm::types::ContentBlockType::ToolUse,
            tool_use_id: Some(tool_use_id.into()),
            tool_name: Some("ExitPlanMode".into()),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::InputJsonDelta {
            index: 1,
            partial_json: "{}".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn guarded_exit_plan_persists_draft_after_allowed_finalization() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::plan_mode::ExitPlanModeTool));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: "guarded-plan-session".into(),
        max_turns: Some(1),
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = "plan".into();
    let mut agent = Agent::new(
        Arc::new(ExitPlanCompletionProvider::default()),
        registry,
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.mode = AgentMode::Plan;
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Auto);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.set_guardrail_action_id(Some("guarded-plan-action".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Allowed));

    agent.process_message("finish the plan").await.unwrap();

    let messages = serde_json::to_string_pretty(&agent.conversation_state().messages).unwrap();
    assert!(messages.contains("# Plan: Guarded Plan"), "{messages}");
    assert!(messages.contains("exit-plan-tool"), "{messages}");
    assert!(messages.contains("tool_result"), "{messages}");
    assert!(!messages.contains("\"is_error\": true"), "{messages}");
    let persisted = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan("guarded-plan-session")
        .unwrap()
        .expect("guarded plan persisted after allowed finalization");
    assert_eq!(persisted.title, "Guarded Plan");
    assert_eq!(persisted.steps.len(), 1);
}

struct ToolBreakCompletionProvider;

#[async_trait::async_trait]
impl LlmProvider for ToolBreakCompletionProvider {
    fn name(&self) -> &str {
        "tool-break-completion"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(5);
        tx.send(StreamEvent::ThinkingDelta {
            index: 0,
            thinking: "draft thinking".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::TextDelta {
            index: 0,
            text: "draft before tool".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentBlockStart {
            index: 1,
            block_type: archon_llm::types::ContentBlockType::ToolUse,
            tool_use_id: Some("tool-1".into()),
            tool_name: Some("missing-tool".into()),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::InputJsonDelta {
            index: 1,
            partial_json: "{}".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn blocked_tool_loop_break_streams_thinking_but_withholds_draft_text_and_reasoning() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        max_turns: Some(1),
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(
        Arc::new(ToolBreakCompletionProvider),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let reasoning_outputs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_reasoning_outputs = Arc::clone(&reasoning_outputs);
    agent.set_record_reasoning_turn_callback(Arc::new(move |payload| {
        captured_reasoning_outputs
            .lock()
            .unwrap()
            .push(payload.assistant_text);
    }));
    agent.set_guardrail_action_id(Some("tool-break-action".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Blocked {
        repair_prompt: "Verification remains missing after tool execution.".into(),
    }));

    let error = agent.process_message("use a tool").await.unwrap_err();

    assert!(matches!(error, AgentLoopError::FinalizationBlocked(_)));
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert!(!events.iter().any(
        |event| matches!(event.inner, AgentEvent::TextDelta(ref text) if text == "draft before tool")
    ));
    assert!(events.iter().any(|event| {
        matches!(event.inner, AgentEvent::TransientThinkingDelta(ref thinking) if thinking == "draft thinking")
    }));
    assert!(
        events
            .iter()
            .any(|event| { matches!(event.inner, AgentEvent::DiscardThinkingPreview) })
    );
    assert!(
        !events
            .iter()
            .any(|event| { matches!(event.inner, AgentEvent::CommitThinkingPreview) })
    );
    assert!(!events.iter().any(|event| {
        matches!(event.inner, AgentEvent::ThinkingDelta(ref thinking) if thinking == "draft thinking")
    }));
    assert!(reasoning_outputs.lock().unwrap().is_empty());
    let persisted_messages = serde_json::to_string(&agent.conversation_state().messages).unwrap();
    assert!(!persisted_messages.contains("draft before tool"));
    assert!(!persisted_messages.contains("draft thinking"));
}

#[tokio::test]
async fn scoped_tool_loop_break_requires_finalization_verdict() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        Arc::new(GuardrailCompletionProvider {
            calls,
            captured_requests: captured_system,
        }),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.set_guardrail_action_id(Some("break-action".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Blocked {
        repair_prompt: "Verification remains missing after tool execution.".into(),
    }));

    let error = agent
        .finalize_tool_loop_break("tool-bearing round")
        .await
        .unwrap_err();

    assert!(matches!(error, AgentLoopError::FinalizationBlocked(_)));
}

include!("plan_approval.rs");
include!("plan_approval_live_smoke.rs");
include!("finalization_preview.rs");
