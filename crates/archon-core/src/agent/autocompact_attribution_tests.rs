use super::*;
use crate::agents::AgentRegistry;
use crate::dispatch::ToolRegistry;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use std::sync::{Arc, Mutex};

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
            text: "summary".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("compaction summaries use streaming")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

#[test]
fn compaction_retry_keeps_session_scope_and_advances_round() {
    let base = serde_json::json!({
        "archon_runtime": {
            "run_id": "session-42",
            "session_id": "session-42",
            "role": "compaction",
            "origin": "auto_compaction",
        }
    });

    let initial = compaction_attempt_attribution(&base, 0);
    let fallback = compaction_attempt_attribution(&base, 1);

    assert_eq!(initial["archon_runtime"]["session_id"], "session-42");
    assert_eq!(fallback["archon_runtime"]["session_id"], "session-42");
    assert_eq!(initial["archon_runtime"]["round"], 0);
    assert_eq!(fallback["archon_runtime"]["round"], 1);
}

#[tokio::test]
async fn auto_compaction_summary_preserves_session_scope_without_denominator() {
    let captured = Arc::new(Mutex::new(None));
    let provider = Arc::new(CapturingSummaryProvider {
        captured: Arc::clone(&captured),
    });
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        provider,
        ToolRegistry::new(),
        AgentConfig {
            session_id: "main-session-42".into(),
            ..AgentConfig::default()
        },
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.messages = (0..8)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            serde_json::json!({
                "role": role,
                "content": format!("main history message {i}: {}", "x".repeat(512)),
            })
        })
        .collect();

    agent
        .run_auto_compaction(CompactAction::Full, true)
        .await
        .expect("compaction summary should succeed");

    let request = captured.lock().unwrap().clone().unwrap();
    let runtime = &request.extra["archon_runtime"];
    assert_eq!(runtime["run_id"], "main-session-42");
    assert_eq!(runtime["session_id"], "main-session-42");
    assert_eq!(runtime["role"], "compaction");
    assert_eq!(runtime["origin"], "auto_compaction");
    assert!(runtime.get("turn").is_none());
    assert_eq!(runtime["round"], 0);
    assert!(runtime.get("effective_denominator").is_none());
}
