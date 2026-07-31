use std::sync::Arc;

use archon_cognitive::{CognitiveConfig, DecisionStore};
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
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    config.session_id = "cognitive-persist-test".to_owned();
    config.context.prompt_cache = false;
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

fn enable_executive_runtime(agent: &mut Agent, root: &std::path::Path) {
    let config = CognitiveConfig {
        enabled: true,
        record_decisions: true,
        record_reflections: true,
        ..CognitiveConfig::default()
    };
    let policy = archon_cognitive::CognitivePolicy {
        enabled: true,
        max_autonomous_risk: "Medium".into(),
        ..archon_cognitive::CognitivePolicy::default()
    };
    agent.set_cognitive_executive(config, policy, root.join(".archon").join("cognitive"));
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

#[tokio::test]
async fn nontrivial_live_turn_records_executive_decision_without_claiming_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());
    enable_executive_runtime(&mut agent, temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    let mut decisions = Vec::new();
    for _ in 0..50 {
        let store =
            archon_cognitive::PersistentCognitiveStore::open(temp.path().join(".archon/cognitive"))
                .expect("cognitive store");
        decisions = DecisionStore::new(
            store.db(),
            temp.path()
                .join(".archon/cognitive/cognitive-decisions.jsonl"),
        )
        .expect("decision store")
        .list_for_session("cognitive-persist-test", 10)
        .expect("executive decisions");
        if !decisions.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].verification_contract.is_some());
    assert!(decisions[0].user_visible_summary.contains("code_change"));
    let mut system = Vec::new();
    agent.inject_turn_requirements(&mut system);
    let executive_prompt = system
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.contains("<cognitive-executive>"))
        .expect("executive advisory prompt");
    assert!(executive_prompt.contains("code_change"));
    assert!(executive_prompt.contains("planning guidance only"));
    assert!(
        !temp
            .path()
            .join(".archon/cognitive/cognitive-reflections.jsonl")
            .exists()
    );
}
