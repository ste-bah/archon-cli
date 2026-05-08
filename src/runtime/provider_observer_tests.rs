use super::*;
use archon_llm::types::Usage;

struct FailingProvider;

#[async_trait]
impl LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "test-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        Err(LlmError::RateLimited {
            retry_after_secs: 30,
        })
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::QuotaExceeded("account exhausted".into()))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

struct CompleteProvider;

#[async_trait]
impl LlmProvider for CompleteProvider {
    fn name(&self) -> &str {
        "complete-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: Vec::new(),
            usage: Usage {
                input_tokens: 3,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            stop_reason: "end_turn".into(),
        })
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        true
    }
}

fn test_db() -> DbInstance {
    let path = format!(
        "/tmp/test-observed-provider-events-{}.db",
        uuid::Uuid::new_v4()
    );
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[tokio::test]
async fn complete_records_start_and_success_events() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(CompleteProvider),
        "direct",
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    );

    observed
        .complete(LlmRequest {
            model: "model-a".into(),
            request_origin: Some("test".into()),
            ..LlmRequest::default()
        })
        .await
        .unwrap();

    let events = archon_learning::runtime_events::list_provider_runtime_events(
        &db,
        Some("complete-provider"),
    )
    .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "request_succeeded");
    assert_eq!(events[1].event_type, "request_started");
    assert_eq!(events[0].model_id.as_deref(), Some("model-a"));
    assert_eq!(events[0].raw_redacted_json["usage"]["output_count"], 5);
}

#[tokio::test]
async fn failures_emit_request_and_limit_events() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(FailingProvider),
        "direct",
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    );

    let error = observed
        .complete(LlmRequest {
            model: "model-a".into(),
            ..LlmRequest::default()
        })
        .await
        .unwrap_err();

    assert!(matches!(error, LlmError::QuotaExceeded(_)));
    let events =
        archon_learning::runtime_events::list_provider_runtime_events(&db, Some("test-provider"))
            .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type, "usage_limit_observed");
    assert_eq!(events[1].event_type, "request_failed");
    assert_eq!(events[2].event_type, "request_started");
    assert_eq!(
        events[0].message.as_deref(),
        Some("provider reported a usage or quota limit")
    );
}

#[test]
fn converts_provider_event_to_learning_record() {
    let record = provider_event_record(
        ProviderRuntimeEvent::new(
            "anthropic",
            "direct",
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
        )
        .with_model("claude-sonnet-4-6")
        .with_redacted_json(serde_json::json!({"authorization": "secret"})),
    );

    assert_eq!(record.event_type, "request_started");
    assert_eq!(record.severity, "debug");
    assert_eq!(record.model_id.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(record.raw_redacted_json["authorization"], "[redacted]");
}
