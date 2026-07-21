use super::*;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::providers::AnthropicProvider;
use archon_llm::types::Secret;
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

fn test_db() -> Arc<DbInstance> {
    crate::command::test_support::registered_learning_test_db("test-observed-provider-events")
}

fn anthropic_provider(identity_mode: IdentityMode) -> Arc<dyn LlmProvider> {
    let identity = IdentityProvider::new(
        identity_mode,
        "session-test".to_string(),
        "device-test".to_string(),
        "account-test".to_string(),
    );
    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("sk-ant-api03-test".to_string())),
        identity,
        None,
    );
    Arc::new(AnthropicProvider::new(client))
}

#[tokio::test(flavor = "current_thread")]
async fn provider_persistence_keeps_current_thread_runtime_responsive_while_write_blocks() {
    use std::sync::mpsc;
    use std::time::Duration;

    let (write_started_tx, write_started_rx) = mpsc::channel();
    let (release_write_tx, release_write_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();

    let coordinator = std::thread::spawn(move || {
        write_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("persistence boundary entered");
        let progressed = progress_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_write_tx
            .send(())
            .expect("release persistence boundary");
        result_tx.send(progressed).expect("report runtime progress");
    });

    let persistence = run_provider_persistence_async_with("test persistence", move || {
        write_started_tx
            .send(())
            .expect("announce persistence boundary");
        release_write_rx
            .recv()
            .expect("release persistence boundary");
    });
    let progress = async move {
        progress_tx.send(()).expect("report runtime progress");
    };
    let (result, ()) = tokio::join!(persistence, progress);
    assert!(result.is_some(), "persistence task must complete");

    coordinator.join().expect("coordinator joins");
    assert!(
        result_rx.recv().expect("runtime progress result"),
        "another Tokio task must progress while provider persistence is blocked"
    );
}

#[tokio::test]
async fn complete_records_start_and_success_events() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(CompleteProvider),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

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
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

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

#[tokio::test]
async fn provider_failures_feed_agent_ledger_when_context_present() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(FailingProvider),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

    let error = observed
        .complete(LlmRequest {
            model: "model-a".into(),
            extra: serde_json::json!({
                "archon_runtime": {
                    "run_id": "session-1",
                    "agent_type": "reviewer",
                    "agent_version": "1.0.0"
                }
            }),
            ..LlmRequest::default()
        })
        .await
        .unwrap_err();

    assert!(matches!(error, LlmError::QuotaExceeded(_)));
    let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        &db, "reviewer",
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].completion_status, "failed");
    assert_eq!(rows[0].agent_version.as_deref(), Some("1.0.0"));
    assert_eq!(rows[0].run_id.as_deref(), Some("session-1"));
    assert!(rows[0].provider_incident_id.as_deref().is_some());
    assert_eq!(rows[0].model_id.as_deref(), Some("model-a"));
    assert_eq!(rows[0].provider_id.as_deref(), Some("test-provider"));
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

#[tokio::test]
async fn observed_events_include_profile_id_when_known() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(CompleteProvider),
        "direct",
        Some("anthropic-oauth".into()),
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

    observed
        .complete(LlmRequest {
            model: "model-a".into(),
            ..LlmRequest::default()
        })
        .await
        .unwrap();

    let events = archon_learning::runtime_events::list_provider_runtime_events(
        &db,
        Some("complete-provider"),
    )
    .unwrap();
    assert_eq!(events[0].profile_id.as_deref(), Some("anthropic-oauth"));
    assert_eq!(events[1].profile_id.as_deref(), Some("anthropic-oauth"));
}

#[tokio::test]
async fn anthropic_spoof_identity_event_is_recorded_on_wrap() {
    let db = test_db();
    let _observed = ObservedLlmProvider::new(
        anthropic_provider(IdentityMode::Spoof {
            version: "2.1.89".to_string(),
            entrypoint: "cli".to_string(),
            betas: vec!["claude-code-20250219".to_string()],
            workload: None,
            anti_distillation: false,
        }),
        "direct",
        Some("anthropic-oauth".into()),
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

    let events =
        archon_learning::runtime_events::list_provider_runtime_events(&db, Some("anthropic"))
            .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "spoof_identity_selected");
    assert_eq!(events[0].profile_id.as_deref(), Some("anthropic-oauth"));
    assert_eq!(events[0].raw_redacted_json["identity_status"], "spoof");
}

#[tokio::test]
async fn anthropic_clean_identity_records_rejected_spoof_event() {
    let db = test_db();
    let _observed = ObservedLlmProvider::new(
        anthropic_provider(IdentityMode::Clean),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;

    let events =
        archon_learning::runtime_events::list_provider_runtime_events(&db, Some("anthropic"))
            .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "spoof_identity_rejected");
    assert_eq!(events[0].reason_code.as_deref(), Some("clean_identity"));
    assert_eq!(events[0].raw_redacted_json["identity_status"], "clean");
}
