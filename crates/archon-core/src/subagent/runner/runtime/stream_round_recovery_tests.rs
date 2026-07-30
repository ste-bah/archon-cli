fn minimal_oversized_messages() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"role":"user","content":"inspect"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"recent-tool","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"recent-tool",
            "content":format!("HEAD{}TAIL", "x".repeat(180_000)),"is_error":false
        }]}),
    ]
}

#[tokio::test]
async fn subagent_pre_stream_no_safe_boundary_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::new());
    let runner = field_recovery_runner(provider.clone());
    let mut messages = minimal_oversized_messages();
    let request = LlmRequest {
        messages: messages.clone(),
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    let mut auto_compact = crate::agent::AutoCompactState::default();
    let mut recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
    let mut emergency_projection_pending = false;
    let mut rate_limit_retried = false;
    let mut last_known_context_tokens = 0;
    let telemetry = crate::agent::autocompact::CompactionTelemetry {
        provider_family: "anthropic",
        wire_shape: "anthropic_messages",
        native_context_window: 1_000_000,
        runtime_context_budget: 1_000_000,
        context_source: "config-override",
        compaction_backend: "anthropic",
    };

    let result = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request,
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("NoSafeBoundary must advance to emergency projection");

    assert_eq!(result.text_content, "recovered");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "initial request and emergency retry");
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) <= 64_000);
}

#[tokio::test]
async fn subagent_mid_stream_no_safe_boundary_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::with_phase(
        FieldFailurePhase::MidStream,
    ));
    let runner = field_recovery_runner(provider.clone());
    let mut messages = minimal_oversized_messages();
    let request = LlmRequest {
        messages: messages.clone(),
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    let mut auto_compact = crate::agent::AutoCompactState::default();
    let mut recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
    let mut emergency_projection_pending = false;
    let mut rate_limit_retried = false;
    let mut last_known_context_tokens = 0;
    let telemetry = crate::agent::autocompact::CompactionTelemetry {
        provider_family: "anthropic",
        wire_shape: "anthropic_messages",
        native_context_window: 1_000_000,
        runtime_context_budget: 1_000_000,
        context_source: "config-override",
        compaction_backend: "anthropic",
    };

    let first = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request.clone(),
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("mid-stream rejection should schedule emergency projection");
    assert!(first.retry_after_compact);

    let recovered = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request,
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("emergency projection should recover after NoSafeBoundary");

    assert_eq!(recovered.text_content, "recovered");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "initial request and emergency retry");
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) <= 64_000);
}

#[tokio::test]
async fn subagent_mid_stream_field_rejection_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::with_phase(
        FieldFailurePhase::MidStream,
    ));
    let runner = field_recovery_runner(provider.clone());
    let mut messages = oversized_recent_messages();
    let canonical = messages.clone();
    let request = LlmRequest {
        messages: messages.clone(),
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    let mut auto_compact = crate::agent::AutoCompactState::default();
    let mut recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
    let mut emergency_projection_pending = false;
    let mut rate_limit_retried = false;
    let mut last_known_context_tokens = 0;
    let telemetry = crate::agent::autocompact::CompactionTelemetry {
        provider_family: "anthropic",
        wire_shape: "anthropic_messages",
        native_context_window: 1_000_000,
        runtime_context_budget: 1_000_000,
        context_source: "config-override",
        compaction_backend: "anthropic",
    };

    let first = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request.clone(),
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("first mid-stream rejection should trigger full compaction");
    assert!(first.retry_after_compact);

    let emergency = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request.clone(),
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("second mid-stream rejection should select emergency projection");
    assert!(emergency.retry_after_compact);

    let recovered = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request,
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("second mid-stream rejection should use emergency projection");

    assert_eq!(recovered.text_content, "recovered");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "initial, full-compaction retry, emergency retry"
    );
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) > 64_000);
    assert!(largest_tool_result_field(&requests[2]) <= 64_000);
    assert_eq!(messages, canonical);
}

#[tokio::test]
async fn subagent_pre_stream_field_rejection_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::new());
    let activity = Arc::new(archon_observability::InMemoryActivitySink::new());
    let mut config = crate::agent::AgentConfig {
        activity_sink: Some(activity.clone()),
        ..crate::agent::AgentConfig::default()
    };
    config.context.preserve_recent_turns = 2;
    config.context.max_tool_result_bytes = 256_000;
    config.context.context_window_override = Some(1_000_000);
    let runner = SubagentRunner::new(
        provider.clone(),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "claude-sonnet-4-6".into(),
        1,
        60,
        Arc::new(config),
        Arc::new(test_identity()),
    );
    let mut messages = vec![
        serde_json::json!({"role":"user","content":"old turn"}),
        serde_json::json!({"role":"assistant","content":"old response"}),
        serde_json::json!({"role":"user","content":"inspect"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"recent-tool","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"recent-tool",
            "content":format!("HEAD{}TAIL", "x".repeat(180_000)),"is_error":false
        }]}),
    ];
    let canonical = messages.clone();
    let request = LlmRequest {
        messages: messages.clone(),
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    let mut auto_compact = crate::agent::AutoCompactState::default();
    let mut recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
    let mut emergency_projection_pending = false;
    let mut rate_limit_retried = false;
    let mut last_known_context_tokens = 0;
    let telemetry = crate::agent::autocompact::CompactionTelemetry {
        provider_family: "anthropic",
        wire_shape: "anthropic_messages",
        native_context_window: 1_000_000,
        runtime_context_budget: 1_000_000,
        context_source: "config-override",
        compaction_backend: "anthropic",
    };

    let result = collect_stream_round(
        &runner,
        &mut messages,
        &mut auto_compact,
        (
            &mut recovery_ladder,
            &mut emergency_projection_pending,
            &mut rate_limit_retried,
            &mut last_known_context_tokens,
        ),
        request,
        (200_000, 1_000_000),
        &telemetry,
    )
    .await
    .expect("emergency projection should recover the field rejection");

    assert_eq!(result.text_content, "recovered");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "initial, full-compaction retry, emergency retry"
    );
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) > 64_000);
    assert!(largest_tool_result_field(&requests[2]) <= 64_000);
    assert_eq!(provider.compaction_calls.load(Ordering::SeqCst), 1);
    assert_eq!(messages, canonical);

    let recovery: Vec<serde_json::Value> = activity
        .events()
        .into_iter()
        .filter_map(|event| serde_json::from_str(&event.message).ok())
        .collect();
    assert_eq!(recovery.len(), 2);
    assert_eq!(recovery[0]["classification"], "tool_result_field");
    assert_eq!(recovery[0]["tier"], "full_compaction");
    assert_eq!(recovery[1]["tier"], "emergency_projection");
    assert_eq!(
        recovery[0]["before_body_bytes"],
        crate::agent::autocompact::request_body_bytes(&requests[0])
    );
    assert_eq!(
        recovery[1]["before_body_bytes"],
        recovery[0]["after_body_bytes"]
    );
    assert!(
        recovery[1]["after_body_bytes"].as_u64().unwrap()
            < recovery[0]["before_body_bytes"].as_u64().unwrap()
    );
    assert_eq!(recovery[1]["reduced"], true);
}
