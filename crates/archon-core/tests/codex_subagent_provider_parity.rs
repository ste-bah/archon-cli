//! Provider-parity regression for Codex-backed subagent loops.
//!
//! This intentionally uses a Codex-named mock provider rather than live network
//! auth. The source of truth is the request stream seen by SubagentRunner:
//! turn 1 emits a tool call, Archon executes the tool, and turn 2 receives the
//! provider-neutral `tool_result` block before returning the final answer.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use archon_core::agent::AgentConfig;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::runner::SubagentRunner;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext, ToolResult};
use tokio::sync::Barrier;

const SENTINEL: &str = "CODEX_SUBAGENT_TOOL_RESULT_OK";

#[derive(Default)]
struct CodexNamedMockProvider {
    call_count: AtomicU32,
    captured_requests: Mutex<Vec<LlmRequest>>,
    first_turn_barrier: Option<Arc<Barrier>>,
}

impl CodexNamedMockProvider {
    fn with_first_turn_barrier(first_turn_barrier: Arc<Barrier>) -> Self {
        Self {
            first_turn_barrier: Some(first_turn_barrier),
            ..Default::default()
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for CodexNamedMockProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "gpt-5.4".into(),
            display_name: "GPT-5.4".into(),
            context_window: 256_000,
        }]
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.captured_requests
            .lock()
            .expect("captured request mutex poisoned")
            .push(request);

        let turn = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        if turn == 1 {
            if let Some(barrier) = &self.first_turn_barrier {
                barrier.wait().await;
            }
        }

        let events = if turn == 1 {
            tool_use_turn()
        } else {
            let received = self
                .captured_requests
                .lock()
                .expect("captured request mutex poisoned")
                .last()
                .and_then(|req| extract_last_tool_result(&req.messages))
                .unwrap_or_else(|| "<missing tool_result>".to_string());
            text_turn(&format!("final answer saw {received}"))
        };

        let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        panic!("CodexNamedMockProvider::complete should not be called by SubagentRunner");
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming | ProviderFeature::ToolUse
        )
    }
}

struct EchoTool;

#[async_trait::async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "Echo"
    }

    fn description(&self) -> &str {
        "Returns the supplied text."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success(
            input
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("<missing text>"),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_named_subagent_executes_tool_and_continues_with_result() {
    let provider = Arc::new(CodexNamedMockProvider::default());
    let runner = make_runner(provider.clone(), "codex-subagent-provider-parity");

    let output = runner
        .run("call Echo with the sentinel")
        .await
        .expect("Codex-named subagent should complete after tool continuation");

    assert!(output.contains(SENTINEL), "final output was: {output}");
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);

    let captured = provider
        .captured_requests
        .lock()
        .expect("captured request mutex poisoned");
    assert_eq!(captured[0].request_origin.as_deref(), Some("subagent"));
    assert_eq!(captured[1].request_origin.as_deref(), Some("subagent"));
    assert_eq!(
        extract_last_tool_result(&captured[1].messages).as_deref(),
        Some(SENTINEL)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_codex_named_subagents_run_concurrently_without_serializing() {
    let barrier = Arc::new(Barrier::new(2));
    let provider_a = Arc::new(CodexNamedMockProvider::with_first_turn_barrier(
        barrier.clone(),
    ));
    let provider_b = Arc::new(CodexNamedMockProvider::with_first_turn_barrier(barrier));
    let runner_a = make_runner(provider_a.clone(), "codex-subagent-provider-parity-a");
    let runner_b = make_runner(provider_b.clone(), "codex-subagent-provider-parity-b");

    let joined = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            runner_a.run("call Echo with the sentinel from A"),
            runner_b.run("call Echo with the sentinel from B")
        )
    })
    .await
    .expect("both subagents must reach the first-turn barrier concurrently");

    let output_a = joined.0.expect("subagent A should complete");
    let output_b = joined.1.expect("subagent B should complete");

    assert!(output_a.contains(SENTINEL), "subagent A output: {output_a}");
    assert!(output_b.contains(SENTINEL), "subagent B output: {output_b}");
    assert_eq!(provider_a.call_count.load(Ordering::SeqCst), 2);
    assert_eq!(provider_b.call_count.load(Ordering::SeqCst), 2);

    for provider in [&provider_a, &provider_b] {
        let captured = provider
            .captured_requests
            .lock()
            .expect("captured request mutex poisoned");
        assert_eq!(captured.len(), 2);
        assert!(
            captured
                .iter()
                .all(|request| request.request_origin.as_deref() == Some("subagent"))
        );
        assert_eq!(
            extract_last_tool_result(&captured[1].messages).as_deref(),
            Some(SENTINEL)
        );
    }
}

fn make_runner(provider: Arc<CodexNamedMockProvider>, session_id: &'static str) -> SubagentRunner {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let registry = Arc::new(registry);

    SubagentRunner::new(
        provider,
        "You are a Codex-backed test subagent.".into(),
        registry.tool_definitions(),
        registry,
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: session_id.into(),
            mode: AgentMode::Normal,
            ..Default::default()
        },
        "gpt-5.4".into(),
        4,
        60,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            session_id.into(),
            String::new(),
            String::new(),
        )),
    )
}

fn tool_use_turn() -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "codex-turn-1".into(),
            model: "gpt-5.4".into(),
            usage: Usage::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some("call_echo_1".into()),
            tool_name: Some("Echo".into()),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: serde_json::json!({ "text": SENTINEL }).to_string(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ]
}

fn text_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "codex-turn-2".into(),
            model: "gpt-5.4".into(),
            usage: Usage::default(),
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

fn extract_last_tool_result(messages: &[serde_json::Value]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.get("role").and_then(|role| role.as_str()) != Some("user") {
            return None;
        }
        message
            .get("content")
            .and_then(|content| content.as_array())
            .and_then(|blocks| {
                blocks.iter().find_map(|block| {
                    if block.get("type").and_then(|kind| kind.as_str()) == Some("tool_result") {
                        block
                            .get("content")
                            .and_then(|content| content.as_str())
                            .map(ToOwned::to_owned)
                    } else {
                        None
                    }
                })
            })
    })
}
