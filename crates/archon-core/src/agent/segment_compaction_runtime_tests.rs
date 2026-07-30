use archon_session::storage::{CompactionSummaryStatus, SessionStore};
use std::sync::atomic::AtomicU32;

struct RuntimeCompactionProvider {
    calls: AtomicU32,
    summary_calls: AtomicU32,
    requests: std::sync::Mutex<Vec<LlmRequest>>,
}

impl RuntimeCompactionProvider {
    fn new() -> Self {
        Self {
            calls: AtomicU32::new(0),
            summary_calls: AtomicU32::new(0),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for RuntimeCompactionProvider {
    fn name(&self) -> &str {
        "runtime-compaction"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "active".into(),
            display_name: "active".into(),
            context_window: 8_192,
        }]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.requests.lock().unwrap().push(request.clone());
        if request.request_origin.as_deref() == Some("compaction_summary") {
            self.summary_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(stream(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "background summary".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(stream(vec![
            StreamEvent::TextDelta {
                index: 0,
                text: "done".into(),
            },
            StreamEvent::MessageStop,
        ])
        .await)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("tests use streaming")
    }
}

async fn stream(events: Vec<StreamEvent>) -> tokio::sync::mpsc::Receiver<StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.unwrap();
    }
    rx
}

fn runtime_agent(
    provider: Arc<RuntimeCompactionProvider>,
    store: Arc<SessionStore>,
    session_id: &str,
) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        model: "active".into(),
        session_id: session_id.into(),
        ..AgentConfig::default()
    };
    config.context.preserve_recent_turns = 1;
    config.context.context_window_override = Some(1_000);
    config.context.output_reserve_tokens = 0;
    config.context.compact_threshold = 0.1;
    let mut agent = Agent::new(
        provider,
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.set_session_store(store);
    agent
}

#[tokio::test]
async fn threshold_request_uses_stored_segment_without_inline_summary_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let body = vec![
        serde_json::json!({"role":"user","content":"early directive"}).to_string(),
        serde_json::json!({"role":"assistant","content":"early answer"}).to_string(),
    ];
    let segment = store
        .close_compaction_segment(&session.id, 0, 1, &body)
        .unwrap();
    let claim = store
        .claim_compaction_segment_summary(&segment.id, "active", "{}")
        .unwrap()
        .expect("claim token");
    store
        .complete_compaction_segment_summary(&segment.id, &claim, "stored summary", 10, 5, 0.01)
        .unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let mut agent = runtime_agent(provider.clone(), store, &session.id);
    agent.state.messages = body
        .iter()
        .map(|message| serde_json::from_str(message).unwrap())
        .chain(std::iter::once(
            serde_json::json!({"role":"user","content":"recent"}),
        ))
        .collect();
    agent.state.last_known_context_tokens = 900;

    agent.process_message("continue").await.unwrap();

    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 0);
    let request = provider.requests.lock().unwrap().first().cloned().unwrap();
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.to_string().contains("stored summary"))
    );
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.to_string().contains("early directive"))
    );
}

#[tokio::test]
async fn short_closed_segment_source_is_present_in_summary_request() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let mut agent = runtime_agent(provider.clone(), store, &session.id);
    agent.state.messages = vec![
        serde_json::json!({"role":"user","content":"must preserve this directive"}),
        serde_json::json!({"role":"assistant","content":"old answer"}),
    ];

    agent.process_message("new turn").await.unwrap();
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    let requests = provider.requests.lock().unwrap();
    let summary_request = requests
        .iter()
        .find(|request| request.request_origin.as_deref() == Some("compaction_summary"))
        .expect("summary request");
    assert!(
        summary_request
            .messages
            .iter()
            .any(|message| message.to_string().contains("must preserve this directive"))
    );
}

#[tokio::test]
async fn completed_turn_closes_one_segment_and_background_summarizes_it_once() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let mut agent = runtime_agent(provider.clone(), store.clone(), &session.id);
    agent.state.messages = vec![
        serde_json::json!({"role":"user","content":"old directive"}),
        serde_json::json!({"role":"assistant","content":"old answer"}),
    ];

    agent.process_message("new turn").await.unwrap();
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    let segments = store.list_compaction_segments(&session.id).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].start_index, 0);
    assert_eq!(segments[0].end_index, 1);
    assert_eq!(
        segments[0].summary_status,
        CompactionSummaryStatus::Succeeded
    );
    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(segments[0].summary_model.as_deref(), Some("active"));
    assert!(
        segments[0]
            .summary_attribution
            .as_deref()
            .is_some_and(|value| value.contains("active_fallback"))
    );
    let telemetry = store.list_compaction_telemetry(&session.id).unwrap();
    assert!(telemetry.iter().any(|record| {
        record.action == "summary_completed"
            && record.payload.contains("duration_ms")
            && record.payload.contains("input_tokens")
            && record.payload.contains("output_tokens")
    }));
    assert_eq!(
        store
            .load_compaction_segment_body(&segments[0].id)
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn restart_recovers_interrupted_summary_without_duplicate_segment() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let body = vec![
        serde_json::json!({"role":"user","content":"old directive"}).to_string(),
        serde_json::json!({"role":"assistant","content":"old answer"}).to_string(),
    ];
    let segment = store
        .close_compaction_segment(&session.id, 0, 1, &body)
        .unwrap();
    assert!(
        store
            .claim_compaction_segment_summary(&segment.id, "active", "{}")
            .unwrap()
            .is_some()
    );
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let mut agent = runtime_agent(provider.clone(), store.clone(), &session.id);
    agent.state.messages = body
        .iter()
        .map(|message| serde_json::from_str(message).unwrap())
        .collect();

    agent.process_message("resume").await.unwrap();
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    let segments = store.list_compaction_segments(&session.id).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(
        segments[0].summary_status,
        CompactionSummaryStatus::Succeeded
    );
    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_marks_malformed_segment_body_failed_without_summary_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let segment = store
        .close_compaction_segment(
            &session.id,
            0,
            1,
            &[
                serde_json::json!({"role":"user","content":"valid"}).to_string(),
                "not-json".into(),
            ],
        )
        .unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());

    let mut agent = runtime_agent(provider.clone(), store.clone(), &session.id);
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 0);
    let stored = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(stored.summary_status, CompactionSummaryStatus::Failed);
    assert!(
        stored
            .summary_failure
            .as_deref()
            .is_some_and(|failure| failure.contains("malformed persisted source"))
    );
}

#[tokio::test]
async fn restart_rejects_structurally_invalid_segment_body_without_summary_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let segment = store
        .close_compaction_segment(
            &session.id,
            0,
            1,
            &[
                serde_json::json!({"role":"user","content":"valid"}).to_string(),
                serde_json::Value::Null.to_string(),
            ],
        )
        .unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());

    let mut agent = runtime_agent(provider.clone(), store.clone(), &session.id);
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 0);
    let stored = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(stored.summary_status, CompactionSummaryStatus::Failed);
    assert!(
        stored
            .summary_failure
            .as_deref()
            .is_some_and(|failure| failure.contains("invalid persisted source message"))
    );
}

#[tokio::test]
async fn live_closure_rejects_structurally_invalid_source_without_provider_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let mut agent = runtime_agent(provider.clone(), store.clone(), &session.id);
    agent.state.messages = vec![
        serde_json::Value::Null,
        serde_json::json!({"role":"user","content":"recent"}),
    ];

    agent.close_completed_compaction_segment("active");
    agent
        .flush_compaction_summaries(std::time::Duration::from_secs(2))
        .await;

    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 0);
    assert!(
        store
            .list_compaction_segments(&session.id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn recall_rejects_structurally_invalid_persisted_source() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let segment = store
        .close_compaction_segment(&session.id, 0, 0, &[serde_json::Value::Null.to_string()])
        .unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let agent = runtime_agent(provider, store, &session.id);

    let error = agent
        .recall_compaction_segment(&segment.id, 1_024)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid persisted source message")
    );
}

#[tokio::test]
async fn authorized_recall_is_provider_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::open(&temp.path().join("sessions.db")).unwrap());
    let session = store.create_session("/tmp", None, "active").unwrap();
    let segment = store
        .close_compaction_segment(
            &session.id,
            0,
            0,
            &[serde_json::json!({
                "role": "user",
                "content": "é".repeat(20_000),
            })
            .to_string()],
        )
        .unwrap();
    let provider = Arc::new(RuntimeCompactionProvider::new());
    let agent = runtime_agent(provider, store, &session.id);

    let recalled = agent.recall_compaction_segment(&segment.id, 1_024).unwrap();

    assert!(
        serde_json::to_vec(&serde_json::Value::String(recalled))
            .unwrap()
            .len()
            <= 1_024
    );
}
