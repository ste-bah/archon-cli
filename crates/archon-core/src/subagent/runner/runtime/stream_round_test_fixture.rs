struct FailingCompactionProvider;

#[async_trait::async_trait]
impl LlmProvider for FailingCompactionProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            return Err(LlmError::RateLimited {
                retry_after_secs: 30,
            });
        }
        unreachable!("fixture only invokes compaction summary")
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("fixture streams")
    }
}

#[tokio::test]
async fn reactive_subagent_compaction_failure_updates_breaker_state() {
    let mut config = crate::agent::AgentConfig::default();
    config.context.context_window_override = Some(1_000_000);
    let runner = SubagentRunner::new(
        Arc::new(FailingCompactionProvider),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "claude-sonnet-4-6".into(),
        1,
        60,
        Arc::new(config),
        Arc::new(test_identity()),
    );
    let mut messages = MessageHistory::new(
        (0..5)
            .map(|index| serde_json::json!({"role":"user","content":format!("turn-{index}")}))
            .collect(),
    );
    let mut auto_compact = crate::agent::AutoCompactState::default();
    let mut last_known_context_tokens = 0;

    let error = compact_messages_for_retry(
        &runner,
        &mut messages,
        &mut auto_compact,
        &mut last_known_context_tokens,
    )
    .await
    .expect_err("rate-limited compaction must fail");

    assert!(matches!(
        error,
        crate::agent::autocompact::CompactionError::Provider(LlmError::RateLimited { .. })
    ));
    assert_eq!(auto_compact.transient_failures, 1);
    assert!(auto_compact.cooldown_until.is_some());
    assert!(!auto_compact.disabled);
}

struct AnthropicTestProvider;

#[derive(Clone, Copy)]
enum FieldFailurePhase {
    PreStream,
    MidStream,
}

struct FieldLimitRecoveryProvider {
    phase: FieldFailurePhase,
    real_calls: AtomicU32,
    compaction_calls: AtomicU32,
    requests: Mutex<Vec<LlmRequest>>,
}

impl FieldLimitRecoveryProvider {
    fn new() -> Self {
        Self::with_phase(FieldFailurePhase::PreStream)
    }

    fn with_phase(phase: FieldFailurePhase) -> Self {
        Self {
            phase,
            real_calls: AtomicU32::new(0),
            compaction_calls: AtomicU32::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for FieldLimitRecoveryProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(stream_events(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Compacted subagent history.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }
        self.real_calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        if largest_tool_result_field(&request) > 64_000 {
            return match self.phase {
                FieldFailurePhase::PreStream => Err(LlmError::Http(
                    "messages.4.content.0.tool_result.content: String should have at most 64000 bytes"
                        .into(),
                )),
                FieldFailurePhase::MidStream => Ok(stream_events(vec![
                    StreamEvent::Error {
                        error_type: "invalid_request_error".into(),
                        message: "messages.4.content.0.tool_result.content: String should have at most 64000 bytes".into(),
                    },
                    StreamEvent::MessageStop,
                ])
                .await),
            };
        }
        Ok(stream_events(vec![
            StreamEvent::TextDelta {
                index: 0,
                text: "recovered".into(),
            },
            StreamEvent::MessageStop,
        ])
        .await)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("tests use streaming")
    }
}

fn largest_tool_result_field(request: &LlmRequest) -> usize {
    request
        .messages
        .iter()
        .filter_map(|message| message.get("content").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|block| {
            block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
        })
        .filter_map(|block| block.get("content").and_then(serde_json::Value::as_str))
        .map(|content| {
            serde_json::to_vec(&serde_json::Value::String(content.to_string()))
                .expect("serialize tool result")
                .len()
        })
        .max()
        .unwrap_or(0)
}

async fn stream_events(events: Vec<StreamEvent>) -> mpsc::Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.expect("send stream event");
    }
    rx
}

struct StalledProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
    dropped: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait::async_trait]
impl LlmProvider for StalledProvider {
    fn name(&self) -> &str {
        "stalled"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(1);
        let dropped = self.dropped.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            tx.closed().await;
            let _ = dropped.send(());
        });
        self.started.lock().unwrap().take().unwrap().send(()).ok();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("stalled provider only streams")
    }
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicTestProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn cache_strategy(&self, _model: &str) -> archon_llm::cache_strategy::CacheStrategy {
        archon_llm::cache_strategy::ANTHROPIC_API
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        unreachable!("projection tests do not open a stream")
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("projection tests do not complete a request")
    }
}

fn runner() -> SubagentRunner {
    let mut config = crate::agent::AgentConfig::default();
    config.context.prompt_cache = true;
    config.context.prompt_cache_conversation = true;
    SubagentRunner::new(
        Arc::new(AnthropicTestProvider),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "claude-sonnet-4-6".into(),
        1,
        60,
        Arc::new(config),
        Arc::new(test_identity()),
    )
}

fn test_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    )
}

fn field_recovery_runner(provider: Arc<FieldLimitRecoveryProvider>) -> SubagentRunner {
    let mut config = crate::agent::AgentConfig::default();
    config.context.preserve_recent_turns = 2;
    config.context.max_tool_result_bytes = 256_000;
    config.context.context_window_override = Some(1_000_000);
    SubagentRunner::new(
        provider,
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "claude-sonnet-4-6".into(),
        1,
        60,
        Arc::new(config),
        Arc::new(test_identity()),
    )
}

fn oversized_recent_messages() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"role":"user","content":"old turn"}),
        serde_json::json!({"role":"assistant","content":"old response"}),
        serde_json::json!({"role":"user","content":"inspect"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"recent-tool","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"recent-tool",
            "content":format!("HEAD{}TAIL", "x".repeat(180_000)),"is_error":false
        }]}),
    ]
}
