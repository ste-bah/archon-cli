use std::sync::Arc;

use archon_llm::provider::{LlmError, LlmProvider, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;

use super::*;

struct QuietLlmProvider;

#[async_trait::async_trait]
impl LlmProvider for QuietLlmProvider {
    fn name(&self) -> &str {
        "quiet"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

fn agent_with_cognitive_store(root: &std::path::Path) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut config = AgentConfig {
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    config.session_id = "cognitive-persist-test".to_owned();
    let mut agent = Agent::new(
        Arc::new(QuietLlmProvider),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
    );
    let store =
        archon_cognitive::PersistentCognitiveStore::open(root.join(".archon").join("cognitive"))
            .expect("cognitive store");
    agent.set_cognitive_store(store);
    agent
}

#[tokio::test]
async fn greeting_turn_records_compact_cognitive_situation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());

    agent
        .process_message("hello")
        .await
        .expect("process message");

    let store = agent.cognitive_store.as_ref().expect("store");
    let store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(store.situation_count().expect("situation count"), 1);
    assert_eq!(store.decision_count().expect("decision count"), 0);
}
