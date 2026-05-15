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
