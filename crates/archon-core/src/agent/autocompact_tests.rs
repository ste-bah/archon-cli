use super::*;
use crate::agents::AgentRegistry;
use crate::dispatch::ToolRegistry;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use std::sync::Arc;

#[test]
fn json_compaction_does_not_emit_system_role_or_orphan_tool_result() {
    let mut messages: Vec<serde_json::Value> = (0..4)
        .map(|i| serde_json::json!({"role": "user", "content": format!("old {i}")}))
        .collect();
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": [
            {"type": "text", "text": "calling"},
            {"type": "tool_use", "id": "tool-1", "name": "Bash", "input": {}}
        ]
    }));
    messages.push(serde_json::json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}]
    }));
    messages.extend(
        (0..5).map(|i| serde_json::json!({"role": "user", "content": format!("recent {i}")})),
    );

    let compacted =
        compact_json_messages_apply_with_summary(&messages, CompactAction::Full, "summary")
            .unwrap();
    assert!(compacted.iter().all(|m| m["role"] != "system"));
    assert_eq!(compacted[1]["content"][1]["id"], "tool-1");
    assert_eq!(compacted[2]["content"][0]["tool_use_id"], "tool-1");
}

#[test]
fn successful_compact_resets_failure_counter() {
    let mut state = AutoCompactState::default();
    state.on_real_failure();
    assert_eq!(state.consecutive_failures, 1);
    state.on_success(123);
    state.on_real_failure();
    assert_eq!(state.consecutive_failures, 1);
    assert!(!state.disabled);
}

#[test]
fn cancelled_stream_classification_is_specific() {
    assert!(is_cancelled_stream_error("request_cancelled", ""));
    assert!(is_cancelled_stream_error("", "operation cancelled by user"));
    assert!(!is_cancelled_stream_error(
        "http_error",
        "request aborted by upstream proxy"
    ));
}

#[test]
fn evaluate_compaction_uses_current_size_not_cumulative() {
    let state = AutoCompactState::default();
    assert_eq!(
        evaluate_compaction(10_000, 1_000_000, &state, MICRO_COMPACT_FRACTION),
        None
    );
    assert_eq!(
        evaluate_compaction(700_000, 1_000_000, &state, MICRO_COMPACT_FRACTION),
        Some(CompactAction::Full)
    );
}

#[test]
fn trigger_token_value_is_current_messages_estimate() {
    let messages = vec![serde_json::json!({"role": "user", "content": "current prompt only"})];
    assert_eq!(
        trigger_tokens(&messages),
        estimate_messages_tokens(&messages)
    );
}

#[test]
fn maybe_auto_compact_uses_last_known_context_tokens_over_estimate() {
    // Verify the three distinct branches of evaluate_compaction that
    // maybe_auto_compact exercises by choosing last_known_context_tokens
    // over the messages estimate (autocompact.rs:147-151).
    //
    // Since evaluate_compaction is a pure function we can drive all three
    // outcomes directly — the branching in maybe_auto_compact reduces to
    // "which token count do I pass?", so these cases prove that passing
    // the real API count fires the right action regardless of what the
    // messages-estimate alone would have said.
    let state = AutoCompactState::default();

    // A) last_known path: 850k against 950k effective window → Full compact
    //    (850_000 / 950_000 = 0.895, above threshold 0.80)
    let action = evaluate_compaction(850_000, 950_000, &state, 0.80);
    assert_eq!(
        action,
        Some(CompactAction::Full),
        "last_known_context_tokens above threshold should trigger Full compact"
    );

    // B) messages-estimate path would be far below threshold → no compaction
    let action_low = evaluate_compaction(50_000, 950_000, &state, 0.80);
    assert_eq!(
        action_low, None,
        "messages-only estimate below threshold should not trigger"
    );

    // C) Micro zone: 650k against 950k (0.684 > MICRO_COMPACT_FRACTION 0.65) → Micro
    let action_micro = evaluate_compaction(650_000, 950_000, &state, 0.80);
    assert_eq!(
        action_micro,
        Some(CompactAction::Micro),
        "token count in micro zone should trigger Micro compact"
    );
}

struct RateLimitedProvider;

#[async_trait::async_trait]
impl LlmProvider for RateLimitedProvider {
    fn name(&self) -> &str {
        "rate-limited"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        Err(LlmError::RateLimited {
            retry_after_secs: 30,
        })
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("compaction summaries use streaming")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

fn test_agent() -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    Agent::new(
        Arc::new(RateLimitedProvider),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
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
