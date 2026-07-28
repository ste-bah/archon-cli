use super::*;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use archon_memory::MemoryGraph;
use std::sync::{Arc, Mutex};

struct CapturingExtractionProvider {
    captured: Arc<Mutex<Option<LlmRequest>>>,
    captured_notify: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl LlmProvider for CapturingExtractionProvider {
    fn name(&self) -> &str {
        "capturing-extraction"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        *self.captured.lock().expect("capture lock") = Some(request);
        self.captured_notify.notify_one();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("memory extraction uses streaming")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

#[tokio::test]
async fn memory_extraction_request_carries_session_attribution() {
    let captured = Arc::new(Mutex::new(None));
    let captured_notify = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(CapturingExtractionProvider {
        captured: Arc::clone(&captured),
        captured_notify: Arc::clone(&captured_notify),
    });
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        provider,
        ToolRegistry::new(),
        AgentConfig {
            session_id: "memory-session-42".into(),
            ..AgentConfig::default()
        },
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.set_memory(Arc::new(MemoryGraph::in_memory().expect("in-memory graph")));
    agent.turn_number = 7;
    agent.state.add_user_message("remember the production fact");

    agent.trigger_memory_extraction();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        captured_notify.notified(),
    )
    .await
    .expect("memory extraction request");

    let request = captured.lock().unwrap().clone().unwrap();
    let runtime = &request.extra["archon_runtime"];
    assert_eq!(runtime["run_id"], "memory-session-42");
    assert_eq!(runtime["session_id"], "memory-session-42");
    assert_eq!(runtime["role"], "memory_extraction");
    assert_eq!(runtime["origin"], "memory_extraction");
    assert_eq!(runtime["turn"], 7);
    assert!(runtime.get("effective_denominator").is_none());
}
