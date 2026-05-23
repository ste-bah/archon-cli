use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;
use archon_video::schema::create_video_schema;
use archon_video::summary::generate_video_summary;
use async_trait::async_trait;
use cozo::DbInstance;
use tokio::sync::mpsc::Receiver;

struct MockLlm {
    fail: bool,
}

#[async_trait]
impl LlmProvider for MockLlm {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        Err(LlmError::Unsupported("stream".into()))
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        if self.fail {
            return Err(LlmError::Http("down".into()));
        }
        Ok(LlmResponse {
            content: vec![serde_json::json!({"text": "Structured video summary"})],
            usage: Usage::default(),
            stop_reason: "end_turn".into(),
        })
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    db
}

fn allowed_policy() -> archon_policy::EffectivePolicy {
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.summary.enabled = true;
    policy.video.summary.allow_llm_summary = true;
    policy.video.summary.provider = "local".into();
    policy
}

#[tokio::test]
async fn enabled_summary_writes_single_video_summary_chunk() {
    let db = db();

    let summary = generate_video_summary(
        &MockLlm { fail: false },
        "video-1",
        "doc-1",
        "spoken evidence",
        "visual evidence",
        9_000,
        &allowed_policy(),
        &db,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(summary, "Structured video summary");
    let chunks = archon_docs::store::list_chunks_for_doc(&db, "doc-1").unwrap();
    assert_eq!(chunks.len(), 1);
    let artifacts = archon_docs::store::list_artifacts_for_doc(&db, "doc-1").unwrap();
    assert_eq!(artifacts[0].artifact_type, "video_summary");
}

#[tokio::test]
async fn disabled_or_failing_summary_does_not_error() {
    let db = db();
    let disabled = archon_policy::EffectivePolicy::default();
    let none = generate_video_summary(
        &MockLlm { fail: false },
        "video-1",
        "doc-1",
        "spoken",
        "visual",
        9_000,
        &disabled,
        &db,
    )
    .await
    .unwrap();
    assert!(none.is_none());

    let failed = generate_video_summary(
        &MockLlm { fail: true },
        "video-1",
        "doc-1",
        "spoken",
        "visual",
        9_000,
        &allowed_policy(),
        &db,
    )
    .await
    .unwrap();
    assert!(failed.is_none());
}
