use super::*;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};

struct FakeProvider;

#[async_trait::async_trait]
impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake-local"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError> {
        Err(LlmError::Unsupported("stream".into()))
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: vec![serde_json::json!({
                "text": r#"{"findings":[{"claim_id":"rqclm_fake","event_kind":"verification_needed","verification_state":"needs_human_review","confidence":0.8,"rationale":"needs source"}]}"#
            })],
            usage: archon_llm::types::Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            stop_reason: "end_turn".into(),
        })
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Local
    }
}

struct CodexLikeProvider;

#[async_trait::async_trait]
impl LlmProvider for CodexLikeProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "gpt-5.4".into(),
            display_name: "GPT-5.4".into(),
            context_window: 1_050_000,
        }]
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError> {
        Err(LlmError::Unsupported("stream".into()))
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported("complete".into()))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Cloud
    }
}

#[tokio::test]
async fn critic_writes_derived_event_and_cost_row() {
    let temp = tempfile::tempdir().unwrap();
    let mut policy = archon_policy::models::EffectivePolicy::default();
    policy.reasoning_quality.allow_llm_critic = true;
    let mut config = archon_core::config::ReasoningQualityCriticConfig::default();
    config.allow_llm = true;
    let event = ReasoningQualityEvent {
        event_id: "rqevt_fake".into(),
        claim_id: "rqclm_fake".into(),
        session_id: "s1".into(),
        canonical_text: "the code does the thing".into(),
        severity_effective: 0.7,
        ..ReasoningQualityEvent::default()
    };

    run_critic(
        CriticRuntime {
            enabled: true,
            config,
            policy,
            provider: Arc::new(FakeProvider),
            model_fallback: "fake-model".into(),
            root: temp.path().to_path_buf(),
            learning_db: None,
            world_root: None,
            feed_world_model: false,
            update_self_trust: false,
        },
        vec![event],
    )
    .await
    .unwrap();

    let store = archon_reasoning_quality::store::ReasoningQualityStore::open(temp.path()).unwrap();
    let events = store.events_for_session("s1").unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == ReasoningEventKind::VerificationNeeded)
    );
    assert!(read_usage_summary(temp.path(), Some("s1")).session_tokens > 0);
}

#[test]
fn default_provider_critic_model_falls_back_when_model_is_not_available() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = archon_core::config::ReasoningQualityCriticConfig::default();
    config.provider = "default".into();
    config.model = "claude-opus-4-7".into();
    let runtime = CriticRuntime {
        enabled: true,
        config,
        policy: archon_policy::models::EffectivePolicy::default(),
        provider: Arc::new(CodexLikeProvider),
        model_fallback: "gpt-5.4".into(),
        root: temp.path().to_path_buf(),
        learning_db: None,
        world_root: None,
        feed_world_model: false,
        update_self_trust: false,
    };

    assert_eq!(critic_model(&runtime), "gpt-5.4");
}
