use super::*;
use archon_llm::types::Usage;

struct StreamProvider {
    events: std::sync::Mutex<Option<Vec<StreamEvent>>>,
}

impl StreamProvider {
    fn new(events: Vec<StreamEvent>) -> Self {
        Self {
            events: std::sync::Mutex::new(Some(events)),
        }
    }
}

#[async_trait]
impl LlmProvider for StreamProvider {
    fn name(&self) -> &str {
        "stream-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let events = self.events.lock().unwrap().take().unwrap_or_default();
        for event in events {
            tx.send(event).await.unwrap();
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("stream test provider must not complete")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        true
    }
}

struct HangingStreamProvider {
    sender: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<StreamEvent>>>,
}

impl HangingStreamProvider {
    fn new() -> Self {
        Self {
            sender: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl LlmProvider for HangingStreamProvider {
    fn name(&self) -> &str {
        "hanging-stream-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(stream_start_usage(13, 0, 0, 0)).await.unwrap();
        *self.sender.lock().unwrap() = Some(tx);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("stream test provider must not complete")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        true
    }
}

fn stream_start_usage(
    input: u64,
    output: u64,
    cache_creation: u64,
    cache_read: u64,
) -> StreamEvent {
    StreamEvent::MessageStart {
        id: "message-1".into(),
        model: "model-a".into(),
        usage: Usage {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cache_creation,
            cache_read_input_tokens: cache_read,
            input_tokens_available: true,
            output_tokens_available: true,
            cache_creation_input_tokens_available: true,
            cache_read_input_tokens_available: true,
        },
    }
}

fn test_db() -> crate::command::test_db::TestDb<std::sync::Arc<cozo::DbInstance>> {
    crate::command::test_support::registered_learning_test_db("test-observed-provider-usage")
}

async fn await_usage_rows(
    db: &Arc<DbInstance>,
) -> Vec<archon_learning::llm_call_usage::LlmCallUsageRecord> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let rows = archon_learning::llm_call_usage::list_llm_call_usage(
                db,
                &archon_learning::llm_call_usage::LlmCallUsageScope::default(),
            )
            .unwrap();
            if !rows.is_empty() {
                return rows;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("logical-call row must be persisted")
}

#[test]
fn observed_request_prefers_runtime_origin_over_transport_origin() {
    let request = LlmRequest {
        model: "model-a".into(),
        request_origin: Some("compaction_summary".into()),
        extra: serde_json::json!({
            "archon_runtime": {
                "origin": "auto_compaction"
            }
        }),
        ..LlmRequest::default()
    };

    let observed = ObservedRequest::from_request("provider-a", &request);

    assert_eq!(observed.origin.as_deref(), Some("auto_compaction"));
}

#[test]
fn observed_request_falls_back_to_transport_origin() {
    let request = LlmRequest {
        model: "model-a".into(),
        request_origin: Some("legacy_origin".into()),
        ..LlmRequest::default()
    };

    let observed = ObservedRequest::from_request("provider-a", &request);

    assert_eq!(observed.origin.as_deref(), Some("legacy_origin"));
}

#[test]
fn context_input_requires_all_version_one_components() {
    let partial = Usage {
        input_tokens: 7,
        input_tokens_available: true,
        ..Usage::default()
    };
    let explicit_zero = Usage {
        input_tokens_available: true,
        cache_creation_input_tokens_available: true,
        cache_read_input_tokens_available: true,
        ..Usage::default()
    };
    let full = Usage {
        input_tokens: 7,
        cache_creation_input_tokens: 2,
        cache_read_input_tokens: 3,
        input_tokens_available: true,
        cache_creation_input_tokens_available: true,
        cache_read_input_tokens_available: true,
        ..Usage::default()
    };

    assert_eq!(context_input_tokens(&partial), None);
    assert_eq!(context_input_tokens(&explicit_zero), Some(0));
    assert_eq!(context_input_tokens(&full), Some(12));
}

#[tokio::test]
async fn stream_success_records_one_usage_row_with_explicit_zero_cache() {
    let db = test_db();
    let observed = observed_stream(
        &db,
        vec![
            stream_start_usage(11, 0, 0, 0),
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: Some(Usage {
                    output_tokens: 7,
                    output_tokens_available: true,
                    ..Usage::default()
                }),
            },
            StreamEvent::MessageStop,
        ],
    )
    .await;

    drain(observed).await;
    let rows = await_usage_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].terminal_status, "succeeded");
    assert_known(&rows[0].input_tokens, 11);
    assert_known(&rows[0].output_tokens, 7);
    assert_known(&rows[0].cache_read_input_tokens, 0);
}

#[tokio::test]
async fn stream_error_then_stop_records_one_failed_usage_row() {
    let db = test_db();
    let observed = observed_stream(
        &db,
        vec![
            stream_start_usage(5, 0, 0, 0),
            StreamEvent::Error {
                error_type: "overloaded".into(),
                message: "retry later".into(),
            },
            StreamEvent::MessageStop,
        ],
    )
    .await;

    drain(observed).await;
    let rows = await_usage_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].terminal_status, "failed");
    assert_known(&rows[0].input_tokens, 5);
}

#[tokio::test]
async fn stream_provider_close_records_partial_usage() {
    let db = test_db();
    let observed = observed_stream(&db, vec![stream_start_usage(9, 0, 0, 0)]).await;

    drain(observed).await;
    let rows = await_usage_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].terminal_status, "closed_without_stop");
    assert_known(&rows[0].input_tokens, 9);
}

#[tokio::test]
async fn stream_consumer_abort_records_partial_usage_once() {
    let db = test_db();
    let observed = ObservedLlmProvider::new(
        Arc::new(HangingStreamProvider::new()),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await;
    let stream = observed.stream(test_request()).await.unwrap();
    drop(stream);

    let rows = await_usage_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].terminal_status, "consumer_closed");
    assert_known(&rows[0].input_tokens, 13);
}

async fn observed_stream(db: &Arc<DbInstance>, events: Vec<StreamEvent>) -> ObservedLlmProvider {
    ObservedLlmProvider::new(
        Arc::new(StreamProvider::new(events)),
        "direct",
        None,
        ProviderRuntimeEventRecorder::with_db(db.clone()),
    )
    .await
}

async fn drain(observed: ObservedLlmProvider) {
    let mut stream = observed.stream(test_request()).await.unwrap();
    while stream.recv().await.is_some() {}
}

fn test_request() -> LlmRequest {
    LlmRequest {
        model: "model-a".into(),
        ..LlmRequest::default()
    }
}

fn assert_known(value: &archon_learning::llm_call_usage::UsageAvailability, expected: u64) {
    assert_eq!(
        value,
        &archon_learning::llm_call_usage::UsageAvailability::Known(expected)
    );
}
