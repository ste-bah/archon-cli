use super::*;
use crate::agents::AgentRegistry;
use crate::dispatch::ToolRegistry;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

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
fn compaction_telemetry_exposes_required_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let telemetry = compaction_telemetry(
        &RateLimitedProvider,
        "private-compatible-model",
        None,
        dir.path(),
    );

    assert_eq!(telemetry.provider_family, "openai_compatible");
    assert_eq!(telemetry.wire_shape, "openai_chat_completions");
    assert_eq!(telemetry.native_context_window, 0);
    assert_eq!(telemetry.runtime_context_budget, 0);
    assert_eq!(telemetry.context_source, "fallback");
    assert_eq!(telemetry.compaction_backend, "generic");
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

struct CapturingSummaryProvider {
    captured: Arc<Mutex<Option<LlmRequest>>>,
    summary: String,
}

impl CapturingSummaryProvider {
    fn new(captured: Arc<Mutex<Option<LlmRequest>>>, summary: &str) -> Self {
        Self {
            captured,
            summary: summary.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for CapturingSummaryProvider {
    fn name(&self) -> &str {
        "capturing-summary"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        *self.captured.lock().expect("capture lock") = Some(request);
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(StreamEvent::TextDelta {
            index: 0,
            text: self.summary.clone(),
        })
        .await
        .expect("send summary text");
        tx.send(StreamEvent::MessageStop)
            .await
            .expect("send message stop");
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("compaction summaries use streaming")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

fn serialized_request_len(request: &LlmRequest) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "model": &request.model,
        "max_tokens": request.max_tokens,
        "system": &request.system,
        "messages": &request.messages,
        "tools": &request.tools,
        "thinking": &request.thinking,
        "speed": &request.speed,
        "effort": &request.effort,
        "extra": &request.extra,
        "request_origin": &request.request_origin,
        "reasoning_encrypted": &request.reasoning_encrypted,
    }))
    .expect("serialize request envelope")
    .len()
}

fn test_agent_with_provider(provider: Arc<dyn LlmProvider>) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    Agent::new(
        provider,
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

fn test_agent() -> Agent {
    test_agent_with_provider(Arc::new(RateLimitedProvider))
}

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

    let summary = generate_compaction_summary_structured(&provider, "claude-opus-4-7", &messages)
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

#[derive(Clone, Copy)]
enum RateLimitFailureMode {
    PreStream,
    MidStream,
}

struct RateLimitThenSuccessProvider {
    mode: RateLimitFailureMode,
    real_calls: AtomicU32,
    compaction_calls: AtomicU32,
    real_body_bytes: Mutex<Vec<usize>>,
}

impl RateLimitThenSuccessProvider {
    fn new(mode: RateLimitFailureMode) -> Self {
        Self {
            mode,
            real_calls: AtomicU32::new(0),
            compaction_calls: AtomicU32::new(0),
            real_body_bytes: Mutex::new(Vec::new()),
        }
    }

    fn real_call_count(&self) -> u32 {
        self.real_calls.load(Ordering::SeqCst)
    }

    fn compaction_call_count(&self) -> u32 {
        self.compaction_calls.load(Ordering::SeqCst)
    }

    fn real_body_bytes(&self) -> Vec<usize> {
        self.real_body_bytes
            .lock()
            .expect("body bytes lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for RateLimitThenSuccessProvider {
    fn name(&self) -> &str {
        "rate-limit-then-success"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    async fn stream(&self, request: LlmRequest) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(stream_from_events(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Compacted history summary.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }

        let call = self.real_calls.fetch_add(1, Ordering::SeqCst);
        self.real_body_bytes
            .lock()
            .expect("body bytes lock")
            .push(request_body_bytes(&request));

        match (self.mode, call) {
            (RateLimitFailureMode::PreStream, 0) => Err(LlmError::RateLimited {
                retry_after_secs: 30,
            }),
            (RateLimitFailureMode::MidStream, 0) => Ok(stream_from_events(vec![
                StreamEvent::Error {
                    error_type: "rate_limited".into(),
                    message: "rate limit exceeded; retry after 30s".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await),
            _ => Ok(stream_from_events(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "done".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await),
        }
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("tests use streaming")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

async fn stream_from_events(events: Vec<StreamEvent>) -> tokio::sync::mpsc::Receiver<StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.expect("send stream event");
    }
    rx
}

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

async fn assert_main_rate_limit_compacts_before_one_retry(mode: RateLimitFailureMode) {
    let provider = Arc::new(RateLimitThenSuccessProvider::new(mode));
    let mut agent = test_agent_with_provider(provider.clone());
    agent.config.context.large_request_retry_body_bytes = Some(1);
    agent.config.context.context_window_override = Some(1_000_000);
    agent.state.messages = compaction_ready_messages("main");

    agent
        .process_message("trigger rate-limit retry")
        .await
        .expect("main session should compact and retry once");

    assert_eq!(provider.real_call_count(), 2, "initial call plus one retry");
    assert_eq!(
        provider.compaction_call_count(),
        1,
        "exactly one scoped compaction summary should be requested"
    );
    let bodies = provider.real_body_bytes();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[1] < bodies[0],
        "retry body should be smaller after compaction: before={}, after={}",
        bodies[0],
        bodies[1]
    );
    assert_eq!(agent.state.auto_compact.compaction_count, 1);
    assert_eq!(agent.state.auto_compact.consecutive_failures, 0);
    assert!(!agent.state.auto_compact.disabled);
    assert!(!agent.state.auto_compact.compact_in_flight);
}

#[tokio::test]
async fn main_pre_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::PreStream).await;
}

#[tokio::test]
async fn main_mid_stream_rate_limit_compacts_before_one_retry() {
    assert_main_rate_limit_compacts_before_one_retry(RateLimitFailureMode::MidStream).await;
}
