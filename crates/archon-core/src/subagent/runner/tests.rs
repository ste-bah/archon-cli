use super::*;
use crate::agent::AgentConfig;
use archon_llm::identity::IdentityMode;
use archon_llm::provider::{LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::types::Usage;
use archon_tools::tool::{PermissionLevel, Tool};
use std::sync::atomic::{AtomicU32, Ordering};
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

mod basic;
mod parallel;
mod progress;
