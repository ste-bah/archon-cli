use std::sync::Arc;

use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;

use crate::agents::AgentRegistry;

use super::*;

struct EmptyProvider;

#[async_trait::async_trait]
impl LlmProvider for EmptyProvider {
    fn name(&self) -> &str {
        "empty"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("test does not invoke provider")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

#[test]
fn real_subtask_executor_requires_shared_session_store() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        archon_session::storage::SessionStore::open(&temp.path().join("sessions.db")).unwrap(),
    );
    let executor = RealSubtaskExecutor::new(
        Arc::new(EmptyProvider),
        temp.path().to_path_buf(),
        "model".into(),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(temp.path()))),
        Arc::clone(&store),
    );

    assert!(Arc::ptr_eq(&executor.session_store, &store));
}
