use super::*;
use archon_llm::provider::{LlmError, LlmRequest};

#[test]
fn aggregate_overflow_without_tool_results_classifies_as_opening_prompt() {
    let request = LlmRequest {
        messages: vec![serde_json::json!({
            "role": "user",
            "content": "x".repeat(200_000),
        })],
        ..LlmRequest::default()
    };
    let error = LlmError::ContextWindowExceeded {
        provider_message: "maximum context length exceeded".into(),
        provider: Some("anthropic".into()),
        model: Some("claude-sonnet-4-6".into()),
    };

    assert_eq!(
        request_pressure_kind_for_request(&error, &request),
        Some(RequestPressureKind::OpeningPrompt)
    );
}

#[test]
fn multi_turn_text_only_overflow_stays_aggregate_context() {
    let request = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "user", "content": "old question"}),
            serde_json::json!({"role": "assistant", "content": "old answer"}),
            serde_json::json!({"role": "user", "content": "x".repeat(200_000)}),
        ],
        ..LlmRequest::default()
    };
    let error = LlmError::ContextWindowExceeded {
        provider_message: "maximum context length exceeded".into(),
        provider: Some("anthropic".into()),
        model: Some("claude-sonnet-4-6".into()),
    };

    assert_eq!(
        request_pressure_kind_for_request(&error, &request),
        Some(RequestPressureKind::AggregateContext)
    );
}

#[test]
fn aggregate_overflow_with_tool_results_stays_aggregate_context() {
    let request = LlmRequest {
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tool-1",
                "content": "x".repeat(200_000),
            }],
        })],
        ..LlmRequest::default()
    };
    let error = LlmError::ContextWindowExceeded {
        provider_message: "maximum context length exceeded".into(),
        provider: Some("anthropic".into()),
        model: Some("claude-sonnet-4-6".into()),
    };

    assert_eq!(
        request_pressure_kind_for_request(&error, &request),
        Some(RequestPressureKind::AggregateContext)
    );
}

#[test]
fn transient_provider_failures_enter_cooldown_without_disabling_compaction() {
    let mut state = AutoCompactState::default();
    for error in [
        CompactionError::Provider(LlmError::RateLimited {
            retry_after_secs: 30,
        }),
        CompactionError::Provider(LlmError::Overloaded),
        CompactionError::Provider(LlmError::Server {
            status: 503,
            message: "temporary outage".into(),
        }),
    ] {
        state.on_failure(&error);
    }

    assert!(!state.disabled);
    assert_eq!(state.structural_failures, 0);
    assert_eq!(state.transient_failures, 3);
    assert!(state.cooldown_until.is_some());
    assert!(!state.should_attempt());
}

#[test]
fn expired_transient_cooldown_allows_later_successful_compaction() {
    let mut state = AutoCompactState::default();
    state.on_failure(&CompactionError::Provider(LlmError::RateLimited {
        retry_after_secs: 30,
    }));
    state.cooldown_until = Some(std::time::Instant::now() - std::time::Duration::from_millis(1));

    assert!(state.should_attempt());

    state.on_success(123);
    assert_eq!(state.transient_failures, 0);
    assert!(state.cooldown_until.is_none());
    assert_eq!(state.compaction_count, 1);
}

#[test]
fn structural_summary_failures_alone_trip_permanent_breaker() {
    let mut state = AutoCompactState::default();
    for _ in 0..MAX_COMPACT_FAILURES {
        state.on_failure(&CompactionError::InvalidSummary("empty summary".into()));
    }

    assert!(state.disabled);
    assert_eq!(state.structural_failures, MAX_COMPACT_FAILURES);
    assert_eq!(state.transient_failures, 0);
}

#[test]
fn ordinary_success_resets_transient_compaction_state() {
    let mut state = AutoCompactState::default();
    state.on_failure(&CompactionError::Provider(LlmError::RateLimited {
        retry_after_secs: 30,
    }));

    state.on_ordinary_success();

    assert_eq!(state.transient_failures, 0);
    assert!(state.cooldown_until.is_none());
    assert!(!state.disabled);
}

#[test]
fn cancelled_compaction_never_advances_breaker() {
    let mut state = AutoCompactState::default();
    state.on_failure(&CompactionError::Cancelled);

    assert_eq!(state.structural_failures, 0);
    assert_eq!(state.transient_failures, 0);
    assert!(!state.disabled);
}

#[test]
fn recovery_ladder_advances_normal_then_emergency_then_exhausted() {
    let mut recovery = RecoveryLadder::default();

    assert_eq!(
        recovery.next(RequestPressureKind::AggregateContext),
        Some(RecoveryTier::FullCompaction)
    );
    assert_eq!(
        recovery.next(RequestPressureKind::AggregateContext),
        Some(RecoveryTier::EmergencyProjection)
    );
    assert_eq!(recovery.next(RequestPressureKind::AggregateContext), None);
    assert_eq!(recovery.attempts(), 2);
}

#[test]
fn recovery_telemetry_records_reduction_and_selected_tier() {
    let telemetry = RecoveryTelemetry::new(
        RequestPressureKind::ToolResultField,
        RecoveryTier::EmergencyProjection,
        900_000,
        100_000,
    );

    assert_eq!(
        telemetry.classification,
        RequestPressureKind::ToolResultField
    );
    assert_eq!(telemetry.tier, RecoveryTier::EmergencyProjection);
    assert_eq!(telemetry.before_body_bytes, 900_000);
    assert_eq!(telemetry.after_body_bytes, 100_000);
    assert!(telemetry.reduced);
    assert!(telemetry.after_estimated_tokens < telemetry.before_estimated_tokens);
}

#[test]
fn recovery_telemetry_serializes_cooldown_visibility() {
    let telemetry = RecoveryTelemetry::new(
        RequestPressureKind::AggregateContext,
        RecoveryTier::FullCompaction,
        900_000,
        400_000,
    )
    .with_cooldown_secs(Some(30));
    let value = serde_json::to_value(telemetry).expect("serialize telemetry");

    assert_eq!(value["cooldown_secs"], 30);
}

#[test]
fn successful_compaction_resets_structural_failure_streak() {
    let mut state = AutoCompactState::default();
    state.on_failure(&CompactionError::InvalidSummary("empty summary".into()));
    state.on_failure(&CompactionError::InvalidSummary("empty summary".into()));

    state.on_success(123);

    assert_eq!(state.structural_failures, 0);
    assert_eq!(state.consecutive_failures, 0);
    assert!(!state.disabled);
}

#[test]
fn no_safe_boundary_does_not_damage_the_structural_breaker() {
    let mut state = AutoCompactState::default();

    state.on_failure(&CompactionError::NoSafeBoundary);

    assert_eq!(state.structural_failures, 0);
    assert_eq!(state.transient_failures, 0);
    assert_eq!(state.consecutive_failures, 0);
    assert!(!state.disabled);
}

#[test]
fn successful_compact_resets_failure_counter() {
    let mut state = AutoCompactState::default();
    state.on_failure(&CompactionError::InvalidSummary("invalid".into()));
    assert_eq!(state.consecutive_failures, 1);
    state.on_success(123);
    state.on_failure(&CompactionError::InvalidSummary("invalid".into()));
    assert_eq!(state.consecutive_failures, 1);
    assert!(!state.disabled);
}
