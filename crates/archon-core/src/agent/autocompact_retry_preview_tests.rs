fn compaction_ready_messages(prefix: &str) -> Vec<serde_json::Value> {
    (0..8)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            serde_json::json!({
                "role": role,
                "content": format!("{prefix} history message {i}: {}", "x".repeat(512)),
            })
        })
        .collect()
}

fn assert_retry_preview_lifecycle(
    rx: &mut tokio::sync::mpsc::Receiver<TimestampedEvent>,
    mode: RateLimitFailureMode,
) {
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    let lifecycle = events
        .iter()
        .filter_map(|event| match &event.inner {
            AgentEvent::TransientThinkingDelta(text) => Some(text.as_str()),
            AgentEvent::DiscardThinkingPreview => Some("discard"),
            AgentEvent::CommitThinkingPreview => Some("commit"),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = match mode {
        RateLimitFailureMode::PreStream => vec!["accepted preview", "commit"],
        RateLimitFailureMode::MidStream | RateLimitFailureMode::ContextPressure => {
            vec!["rejected preview", "discard", "accepted preview", "commit"]
        }
    };
    assert_eq!(lifecycle, expected);
}

#[tokio::test]
async fn summary_request_stays_within_hard_input_budget_for_five_large_messages() {
    let captured: Arc<Mutex<Option<LlmRequest>>> = Arc::new(Mutex::new(None));
    let provider = CapturingSummaryProvider::new(Arc::clone(&captured), "summary");
    let messages: Vec<serde_json::Value> = (0..5)
        .map(|index| {
            serde_json::json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": "x".repeat(100_000),
            })
        })
        .collect();

    generate_segment_summary_with_usage(&provider, "active", &messages, serde_json::json!({}))
        .await
        .expect("summary should succeed");

    let request = captured.lock().unwrap().clone().expect("summary request");
    let input_bytes = serde_json::to_vec(&request.messages)
        .expect("serialize summary request messages")
        .len();
    assert!(
        input_bytes <= COMPACTION_INPUT_BUDGET_BYTES,
        "summary input bytes {input_bytes} exceed budget {COMPACTION_INPUT_BUDGET_BYTES}"
    );
}

#[tokio::test]
async fn configured_request_pressure_does_not_mutate_canonical_history() {
    let provider = Arc::new(RateLimitThenSuccessProvider::new(
        RateLimitFailureMode::PreStream,
    ));
    provider.real_calls.store(1, Ordering::SeqCst);
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        provider.clone(),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.set_guardrail_action_id(Some("pressure".into()));
    agent.set_turn_finalization_callback(Arc::new(|_, _| TurnFinalizationVerdict::Allowed));
    agent.config.context.rate_limit_pressure_body_bytes = Some(1);
    agent.config.context.rate_limit_pressure_tokens = Some(1);
    agent.state.messages = compaction_ready_messages("canonical");
    let canonical = agent.state.messages.clone();

    agent.process_message("pressure turn").await.unwrap();

    assert_eq!(provider.real_call_count(), 2);
    assert_eq!(provider.compaction_call_count(), 0);
    assert_eq!(agent.state.messages[..canonical.len()], canonical);
}

#[tokio::test]
async fn main_pre_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::PreStream).await;
}

#[tokio::test]
async fn main_stream_context_pressure_retries_compacted_request_without_mutating_history() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::ContextPressure).await;
}

#[tokio::test]
async fn main_mid_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::MidStream).await;
}
