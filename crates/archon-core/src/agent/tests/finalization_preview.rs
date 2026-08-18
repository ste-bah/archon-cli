#[tokio::test]
async fn unscoped_turn_keeps_streaming_text_with_callback_installed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_system = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
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
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Allowed));

    agent.process_message("implement feature").await.unwrap();

    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::TextDelta(ref text) if text == "final answer"))
            .count(),
        1
    );
}

#[tokio::test]
async fn allowed_finalization_emits_turn_complete_once() {
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
    agent.set_guardrail_action_id(Some("allowed-action".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Allowed));

    agent.process_message("implement feature").await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::TurnComplete { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::TextDelta(ref text) if text == "final answer"))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::TransientThinkingDelta(ref thinking) if thinking == "draft thinking"))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.inner, AgentEvent::CommitThinkingPreview))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::DiscardThinkingPreview))
    );
    let lifecycle = events
        .iter()
        .filter_map(|event| match event.inner {
            AgentEvent::TransientThinkingDelta(_) => Some("preview"),
            AgentEvent::CommitThinkingPreview => Some("commit"),
            AgentEvent::TextDelta(_) => Some("text"),
            AgentEvent::TurnComplete { .. } => Some("complete"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lifecycle, ["preview", "commit", "text", "complete"]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.inner, AgentEvent::ThinkingDelta(_)))
    );
    assert_eq!(
        agent
            .conversation_state()
            .messages
            .iter()
            .filter(|message| {
                message["role"] == "assistant"
                    && message["content"].to_string().contains("final answer")
            })
            .count(),
        1
    );
}
