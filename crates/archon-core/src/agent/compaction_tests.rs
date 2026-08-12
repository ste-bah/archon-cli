use super::*;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use std::sync::Mutex;

struct FailingSummaryProvider;

#[async_trait::async_trait]
impl LlmProvider for FailingSummaryProvider {
    fn name(&self) -> &str {
        "failing-summary"
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
        unreachable!("manual compaction uses streaming summaries")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

struct CapturingSummaryProvider {
    captured: Arc<Mutex<Option<LlmRequest>>>,
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
            text: "Manual path summary.".into(),
        })
        .await
        .expect("send summary text");
        tx.send(StreamEvent::MessageStop)
            .await
            .expect("send message stop");
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("manual compaction uses streaming summaries")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

fn test_agent_with_provider(provider: Arc<dyn LlmProvider>) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
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

fn serialized_request_len(request: &LlmRequest) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "model": &request.model,
        "max_tokens": request.max_tokens,
        "system": &request.system,
        "messages": &request.messages,
        "tools": request.tools.as_ref(),
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

fn test_agent() -> Agent {
    test_agent_with_provider(Arc::new(FailingSummaryProvider))
}

#[tokio::test]
async fn manual_compact_reports_summary_failure_without_synthetic_fallback() {
    let mut agent = test_agent();
    agent.state.messages = (0..6)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();

    let status = agent.compact(Some("micro")).await;

    assert!(matches!(status, ManualCompactOutcome::Failed { .. }));
    assert!(
        status
            .status()
            .contains("Compaction failed: provider summary failed")
    );
    assert!(status.status().contains("rate limited: retry after 30s"));
    assert!(!status.status().contains("Compacted conversation"));
    assert_eq!(agent.state.messages.len(), 6);
}

/// AC#4 deviation guard, half one: bare `/compact` stays thresholded.
///
/// The issue asked for bare `/compact` to compact at any usage; the
/// shipped decision kept it backward-compatible with `/compact auto`
/// (docs/reference/config.md `manual_compact_force_strategy`). Nothing
/// exercised `compact(None)` before, so the thresholded branch — and
/// the status line that has to point the user at `/compact force` —
/// were both untested.
#[tokio::test]
async fn bare_compact_below_threshold_reports_force_escape_hatch() {
    let captured = Arc::new(Mutex::new(None));
    let mut agent = test_agent_with_provider(Arc::new(CapturingSummaryProvider {
        captured: Arc::clone(&captured),
    }));
    // Six tiny messages against the default context window is far below
    // the 60 % `select_strategy` floor.
    agent.state.messages = (0..6)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();

    let outcome = agent.compact(None).await;

    assert!(
        matches!(outcome, ManualCompactOutcome::BelowThreshold { .. }),
        "bare /compact must stay thresholded; got {outcome:?}"
    );
    assert!(
        outcome.status().contains("/compact force"),
        "the below-threshold status must name the escape hatch; got {:?}",
        outcome.status()
    );
    assert_eq!(
        agent.state.messages.len(),
        6,
        "below-threshold /compact must not touch the conversation"
    );
    assert!(
        captured.lock().expect("capture lock").is_none(),
        "below-threshold /compact must not call the summary provider"
    );
}

/// AC#4 deviation guard, half two: `/compact force` compacts anyway.
///
/// Same conversation as the test above — identical usage ratio, so the
/// only variable is the subcommand.
#[tokio::test]
async fn forced_compact_below_threshold_compacts_anyway() {
    let captured = Arc::new(Mutex::new(None));
    let mut agent = test_agent_with_provider(Arc::new(CapturingSummaryProvider {
        captured: Arc::clone(&captured),
    }));
    agent.state.messages = (0..6)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();
    assert_eq!(
        agent.config.context.manual_compact_force_strategy, "micro",
        "fixture assumes the shipped default force strategy"
    );

    let outcome = agent.compact(Some("force")).await;

    assert!(
        matches!(outcome, ManualCompactOutcome::Compacted { .. }),
        "/compact force must compact below threshold; got {outcome:?}"
    );
    assert!(
        outcome.status().contains("micro"),
        "forced compaction should report the configured strategy; got {:?}",
        outcome.status()
    );
    assert!(
        captured.lock().expect("capture lock").is_some(),
        "/compact force must reach the summary provider"
    );
}

#[tokio::test]
async fn manual_compact_path_pre_trims_huge_history() {
    let captured = Arc::new(Mutex::new(None));
    let provider = CapturingSummaryProvider {
        captured: Arc::clone(&captured),
    };
    let mut agent = test_agent_with_provider(Arc::new(provider));
    agent.state.messages = (0..200)
        .map(|i| {
            serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": "x".repeat(10_000),
            })
        })
        .collect();

    let status = agent.compact(Some("micro")).await;

    assert!(matches!(status, ManualCompactOutcome::Compacted { .. }));
    assert!(status.status().contains("Compacted conversation"));
    let request = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("manual compact should call provider");
    let body_len = serialized_request_len(&request);
    assert!(
        body_len <= 640_000,
        "manual compact body should be bounded; got {}",
        body_len
    );
}
