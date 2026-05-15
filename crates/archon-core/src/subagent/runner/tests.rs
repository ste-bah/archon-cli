use super::*;
use crate::agent::AgentConfig;
use archon_llm::identity::IdentityMode;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::types::Usage;
use archon_tools::tool::{PermissionLevel, Tool};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Mock provider that returns pre-configured responses.
struct MockProvider {
    responses: std::sync::Mutex<Vec<Vec<StreamEvent>>>,
    call_count: AtomicU32,
}

impl MockProvider {
    fn new(responses: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, archon_llm::provider::LlmError> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst) as usize;
        let events = {
            let mut responses = self.responses.lock().unwrap();
            if idx < responses.len() {
                responses[idx].drain(..).collect::<Vec<_>>()
            } else {
                vec![
                    StreamEvent::MessageStart {
                        id: "msg-end".into(),
                        model: "mock".into(),
                        usage: Usage {
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                        },
                    },
                    StreamEvent::ContentBlockStart {
                        index: 0,
                        block_type: ContentBlockType::Text,
                        tool_use_id: None,
                        tool_name: None,
                    },
                    StreamEvent::TextDelta {
                        index: 0,
                        text: "(done)".into(),
                    },
                    StreamEvent::ContentBlockStop { index: 0 },
                    StreamEvent::MessageStop,
                ]
            }
        }; // MutexGuard dropped here

        let (tx, rx) = mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(
        &self,
        _request: LlmRequest,
    ) -> Result<LlmResponse, archon_llm::provider::LlmError> {
        unimplemented!()
    }
}

fn text_response(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "msg-1".into(),
            model: "mock".into(),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: text.into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ]
}

fn tool_use_response(tool_id: &str, tool_name: &str, input_json: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "msg-tool".into(),
            model: "mock".into(),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 20,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some(tool_id.into()),
            tool_name: Some(tool_name.into()),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: input_json.into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ]
}

fn make_runner(provider: Arc<dyn LlmProvider>, max_turns: u32) -> SubagentRunner {
    make_runner_with_config(provider, max_turns, AgentConfig::default())
}

fn make_runner_with_config(
    provider: Arc<dyn LlmProvider>,
    max_turns: u32,
    agent_config: AgentConfig,
) -> SubagentRunner {
    let registry = Arc::new(crate::dispatch::create_default_registry(
        std::env::current_dir().unwrap_or_default(),
        None,
    ));
    let tool_defs = registry.tool_definitions();
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test-session".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    };
    SubagentRunner::new(
        provider,
        "You are a test subagent.".into(),
        tool_defs,
        registry,
        ctx,
        "mock-model".into(),
        max_turns,
        300,
        Arc::new(agent_config),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    )
}

#[derive(Clone, Copy)]
enum RateLimitFailureMode {
    PreStream,
    MidStream,
}

struct RateLimitThenSuccessProvider {
    mode: RateLimitFailureMode,
    real_calls: AtomicU32,
    compaction_calls: AtomicU32,
    real_body_bytes: Mutex<Vec<usize>>,
    real_origins: Mutex<Vec<Option<String>>>,
}

impl RateLimitThenSuccessProvider {
    fn new(mode: RateLimitFailureMode) -> Self {
        Self {
            mode,
            real_calls: AtomicU32::new(0),
            compaction_calls: AtomicU32::new(0),
            real_body_bytes: Mutex::new(Vec::new()),
            real_origins: Mutex::new(Vec::new()),
        }
    }

    fn real_call_count(&self) -> u32 {
        self.real_calls.load(Ordering::SeqCst)
    }

    fn compaction_call_count(&self) -> u32 {
        self.compaction_calls.load(Ordering::SeqCst)
    }

    fn real_body_bytes(&self) -> Vec<usize> {
        self.real_body_bytes
            .lock()
            .expect("body bytes lock")
            .clone()
    }

    fn real_origins(&self) -> Vec<Option<String>> {
        self.real_origins.lock().expect("origin lock").clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for RateLimitThenSuccessProvider {
    fn name(&self) -> &str {
        "rate-limit-then-success"
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
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(stream_from_events(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Compacted subagent history summary.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }

        let call = self.real_calls.fetch_add(1, Ordering::SeqCst);
        self.real_body_bytes
            .lock()
            .expect("body bytes lock")
            .push(crate::agent::autocompact::request_body_bytes(&request));
        self.real_origins
            .lock()
            .expect("origin lock")
            .push(request.request_origin.clone());

        match (self.mode, call) {
            (RateLimitFailureMode::PreStream, 0) => Err(LlmError::RateLimited {
                retry_after_secs: 30,
            }),
            (RateLimitFailureMode::MidStream, 0) => Ok(stream_from_events(vec![
                StreamEvent::Error {
                    error_type: "rate_limited".into(),
                    message: "rate limit exceeded; retry after 30s".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await),
            _ => Ok(stream_from_events(text_response("done")).await),
        }
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("tests use streaming")
    }
}

async fn stream_from_events(events: Vec<StreamEvent>) -> tokio::sync::mpsc::Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.expect("send stream event");
    }
    rx
}

fn compaction_ready_messages(prefix: &str) -> Vec<serde_json::Value> {
    (0..8)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            serde_json::json!({
                "role": role,
                "content": format!("{prefix} history message {i}: {}", "x".repeat(512)),
            })
        })
        .collect()
}

async fn assert_subagent_rate_limit_compacts_before_one_retry(mode: RateLimitFailureMode) {
    let provider = Arc::new(RateLimitThenSuccessProvider::new(mode));
    let mut config = AgentConfig::default();
    config.context.large_request_retry_body_bytes = Some(1);
    config.context.context_window_override = Some(1_000_000);
    let mut runner = make_runner_with_config(provider.clone(), 3, config);
    runner.set_initial_messages(compaction_ready_messages("subagent"));

    let output = runner
        .run("trigger rate-limit retry")
        .await
        .expect("subagent should compact and retry once");

    assert_eq!(output, "done");
    assert_eq!(provider.real_call_count(), 2, "initial call plus one retry");
    assert_eq!(
        provider.compaction_call_count(),
        1,
        "exactly one subagent-scoped compaction summary should be requested"
    );
    assert_eq!(
        provider.real_origins(),
        vec![Some("subagent".into()), Some("subagent".into())],
        "rate-limit retry must stay scoped to the subagent request path"
    );
    let bodies = provider.real_body_bytes();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[1] < bodies[0],
        "retry body should be smaller after subagent compaction: before={}, after={}",
        bodies[0],
        bodies[1]
    );
}

#[tokio::test]
async fn subagent_pre_stream_rate_limit_compacts_own_history_before_one_retry() {
    assert_subagent_rate_limit_compacts_before_one_retry(RateLimitFailureMode::PreStream).await;
}

#[tokio::test]
async fn subagent_mid_stream_rate_limit_compacts_own_history_before_one_retry() {
    assert_subagent_rate_limit_compacts_before_one_retry(RateLimitFailureMode::MidStream).await;
}

mod basic;
mod parallel;
mod progress;
