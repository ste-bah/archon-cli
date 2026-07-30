struct CapturingLlmProvider {
    captured: Arc<std::sync::Mutex<Vec<LlmRequest>>>,
}

#[derive(Clone, Copy)]
enum FieldFailurePhase {
    BeforeOpen,
    DuringStream,
    DuringThenBeforeOpen,
}

struct FieldLimitRecoveryProvider {
    phase: FieldFailurePhase,
    real_calls: std::sync::atomic::AtomicU32,
    compaction_calls: std::sync::atomic::AtomicU32,
    requests: std::sync::Mutex<Vec<LlmRequest>>,
}

impl FieldLimitRecoveryProvider {
    fn new() -> Self {
        Self::with_phase(FieldFailurePhase::BeforeOpen)
    }

    fn with_phase(phase: FieldFailurePhase) -> Self {
        Self {
            phase,
            real_calls: std::sync::atomic::AtomicU32::new(0),
            compaction_calls: std::sync::atomic::AtomicU32::new(0),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn real_requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for FieldLimitRecoveryProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            self.compaction_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(test_stream(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Compacted history summary.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }

        let call = self
            .real_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.requests
            .lock()
            .expect("request lock")
            .push(request.clone());
        if largest_tool_result_field(&request) > 64_000 {
            return match self.phase {
                FieldFailurePhase::BeforeOpen => Err(LlmError::Http(
                    "messages.9.content.0.tool_result.content: String should have at most 64000 bytes"
                        .into(),
                )),
                FieldFailurePhase::DuringStream => Ok(test_stream(vec![
                    StreamEvent::Error {
                        error_type: "invalid_request_error".into(),
                        message: "messages.9.content.0.tool_result.content: String should have at most 64000 bytes".into(),
                    },
                    StreamEvent::MessageStop,
                ])
                .await),
                FieldFailurePhase::DuringThenBeforeOpen if call == 0 => Ok(test_stream(vec![
                    StreamEvent::Error {
                        error_type: "invalid_request_error".into(),
                        message: "messages.9.content.0.tool_result.content: String should have at most 64000 bytes".into(),
                    },
                    StreamEvent::MessageStop,
                ])
                .await),
                FieldFailurePhase::DuringThenBeforeOpen => Err(LlmError::Http(
                    "messages.9.content.0.tool_result.content: String should have at most 64000 bytes"
                        .into(),
                )),
            };
        }

        Ok(test_stream(vec![
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

async fn test_stream(events: Vec<StreamEvent>) -> tokio::sync::mpsc::Receiver<StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.expect("send stream event");
    }
    rx
}

#[async_trait::async_trait]
impl LlmProvider for CapturingLlmProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_anthropic_message_caching(&self) -> bool {
        true
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.captured.lock().unwrap().push(request);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

async fn main_field_recovery_agent(
    provider: Arc<FieldLimitRecoveryProvider>,
    messages: Vec<serde_json::Value>,
    activity_sink: Option<Arc<dyn archon_observability::AgentActivitySink>>,
) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        activity_sink,
        ..AgentConfig::default()
    };
    config.context.preserve_recent_turns = 2;
    config.context.max_tool_result_bytes = 256_000;
    config.context.context_window_override = Some(1_000_000);
    let mut agent = Agent::new(
        provider,
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.messages = messages;
    agent
}

#[tokio::test]
async fn no_safe_boundary_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::new());
    let messages = vec![
        serde_json::json!({"role":"user","content":"inspect"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"recent-tool","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"recent-tool",
            "content":format!("HEAD{}TAIL", "x".repeat(180_000)),"is_error":false
        }]}),
    ];
    let mut agent = main_field_recovery_agent(provider.clone(), messages, None).await;

    agent
        .process_message("continue")
        .await
        .expect("NoSafeBoundary must advance to emergency projection");

    let requests = provider.real_requests();
    assert_eq!(requests.len(), 2, "initial request and emergency retry");
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) <= 64_000);
}

#[tokio::test]
async fn main_mid_stream_no_safe_boundary_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::with_phase(
        FieldFailurePhase::DuringStream,
    ));
    let messages = vec![
        serde_json::json!({"role":"user","content":"inspect"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"recent-tool","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"recent-tool",
            "content":format!("HEAD{}TAIL", "x".repeat(180_000)),"is_error":false
        }]}),
    ];
    let mut agent = main_field_recovery_agent(provider.clone(), messages, None).await;

    agent
        .process_message("continue")
        .await
        .expect("mid-stream NoSafeBoundary must advance to emergency projection");

    let requests = provider.real_requests();
    assert_eq!(requests.len(), 2, "initial request and emergency retry");
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) <= 64_000);
}

#[tokio::test]
async fn main_mid_stream_field_rejection_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::with_phase(
        FieldFailurePhase::DuringStream,
    ));
    let activity = Arc::new(archon_observability::InMemoryActivitySink::new());
    let messages = vec![
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
    ];
    let canonical = messages.clone();
    let mut agent =
        main_field_recovery_agent(provider.clone(), messages, Some(activity.clone())).await;

    agent
        .process_message("continue")
        .await
        .expect("mid-stream emergency projection should recover the field rejection");

    let requests = provider.real_requests();
    assert_eq!(
        requests.len(),
        3,
        "initial, full-compaction retry, emergency retry"
    );
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) > 64_000);
    assert!(largest_tool_result_field(&requests[2]) <= 64_000);
    assert_eq!(
        &agent.state.messages[..canonical.len()],
        canonical.as_slice()
    );
    let recovery: Vec<serde_json::Value> = activity
        .events()
        .into_iter()
        .filter_map(|event| serde_json::from_str(&event.message).ok())
        .collect();
    assert_eq!(recovery.len(), 2);
    assert_eq!(recovery[0]["tier"], "full_compaction");
    assert_eq!(recovery[1]["tier"], "emergency_projection");
}

#[tokio::test]
async fn main_compacted_retry_pre_stream_rejection_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::with_phase(
        FieldFailurePhase::DuringThenBeforeOpen,
    ));
    let messages = vec![
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
    ];
    let canonical = messages.clone();
    let mut agent = main_field_recovery_agent(provider.clone(), messages, None).await;

    agent
        .process_message("continue")
        .await
        .expect("pre-stream rejection of compacted retry should advance to emergency projection");

    let requests = provider.real_requests();
    assert_eq!(requests.len(), 3);
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) > 64_000);
    assert!(largest_tool_result_field(&requests[2]) <= 64_000);
    assert_eq!(
        &agent.state.messages[..canonical.len()],
        canonical.as_slice()
    );
}

#[tokio::test]
async fn main_pre_stream_field_rejection_advances_to_emergency_projection() {
    let provider = Arc::new(FieldLimitRecoveryProvider::new());
    let activity = Arc::new(archon_observability::InMemoryActivitySink::new());
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        activity_sink: Some(activity.clone()),
        ..AgentConfig::default()
    };
    config.context.preserve_recent_turns = 2;
    config.context.max_tool_result_bytes = 256_000;
    config.context.context_window_override = Some(1_000_000);
    let mut agent = Agent::new(
        provider.clone(),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.messages = vec![
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
    ];
    let canonical = agent.state.messages.clone();

    agent
        .process_message("continue")
        .await
        .expect("emergency projection should recover the field rejection");

    let requests = provider.real_requests();
    assert_eq!(
        requests.len(),
        3,
        "initial, full-compaction retry, emergency retry"
    );
    assert!(largest_tool_result_field(&requests[0]) > 64_000);
    assert!(largest_tool_result_field(&requests[1]) > 64_000);
    assert!(largest_tool_result_field(&requests[2]) <= 64_000);
    assert_eq!(
        provider
            .compaction_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        &agent.state.messages[..canonical.len()],
        canonical.as_slice()
    );

    let recovery: Vec<serde_json::Value> = activity
        .events()
        .into_iter()
        .filter_map(|event| serde_json::from_str(&event.message).ok())
        .collect();
    assert_eq!(recovery.len(), 2);
    assert_eq!(recovery[0]["classification"], "tool_result_field");
    assert_eq!(recovery[0]["tier"], "full_compaction");
    assert_eq!(recovery[1]["tier"], "emergency_projection");
    assert_eq!(recovery[1]["reduced"], true);
}

include!("tool_result_persistence.rs");
