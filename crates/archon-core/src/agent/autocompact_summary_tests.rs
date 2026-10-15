use super::*;

#[tokio::test]
async fn generate_compaction_summary_pre_trims_huge_history_bounds_body() {
    let captured: Arc<Mutex<Option<LlmRequest>>> = Arc::new(Mutex::new(None));
    let provider = CapturingSummaryProvider::new(Arc::clone(&captured), "Synthesised summary.");
    let messages: Vec<serde_json::Value> = (0..200)
        .map(|i| {
            serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": "x".repeat(10_000),
            })
        })
        .collect();

    let summary = generate_compaction_summary_structured(
        &provider,
        "claude-opus-4-7",
        &messages,
        serde_json::Value::Null,
        2_048,
    )
    .await
    .expect("summary should succeed");

    assert_eq!(summary, "Synthesised summary.");
    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("provider should capture request");
    assert_eq!(request.speed, None);
    assert_eq!(request.effort, None);
    let body_len = serialized_request_len(&request);
    assert!(
        body_len <= 2 * COMPACTION_INPUT_BUDGET_BYTES,
        "captured compaction body should be bounded near {COMPACTION_INPUT_BUDGET_BYTES}; got {}",
        body_len
    );
}

#[tokio::test]
async fn proactive_compaction_provider_rate_limit_is_soft_failure() {
    let mut agent = test_agent();
    let original_messages: Vec<serde_json::Value> = (0..6)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();
    agent.state.messages = original_messages.clone();

    for expected in 1..=3 {
        agent
            .run_auto_compaction(CompactAction::Full, false)
            .await
            .expect("proactive provider failure should soft-fail");
        assert_eq!(agent.state.auto_compact.consecutive_failures, expected);
        assert_eq!(agent.state.messages, original_messages);
    }
    assert!(!agent.state.auto_compact.should_attempt());
}

#[tokio::test]
async fn reactive_compaction_provider_error_remains_fatal() {
    let mut agent = test_agent();
    agent.state.messages = (0..6)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();

    let err = agent.force_reactive_compact().await.unwrap_err();

    assert!(err.to_string().contains("auto-compaction failed"));
    assert_eq!(agent.state.auto_compact.consecutive_failures, 1);
}
