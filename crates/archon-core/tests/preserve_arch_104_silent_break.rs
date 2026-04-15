//! TASK-AGS-105 regression guard: the TASK-AGS-104 silent break.
//!
//! TASK-AGS-104 introduced a tool-owned spawn site where `AgentTool::execute`
//! unconditionally returned `{agent_id, status: "spawned"}`. The agent loop's
//! `handle_subagent_result` used to re-parse that marker and run the subagent
//! — but the parser expected a bare `SubagentRequest`, not a spawn marker, so
//! the foreground path silently returned the marker to the LLM instead of the
//! real subagent text. TASK-AGS-105 fixes this by routing the foreground path
//! through `SubagentExecutor::run_to_completion` and returning the real text.
//!
//! THIS TEST DRIVES THE FULL `Agent::process_message` DISPATCH LOOP. The
//! silent-break surface is the tool_use → tool_result → next-turn LLM input
//! seam, which only exists inside the agent loop. A test that calls
//! `AgentTool::execute` directly does NOT guard that seam — it only guards
//! the tool return value. So this test:
//!
//!   1. Installs a `FixedStringExecutor` via OnceLock that returns a unique
//!      sentinel string from `run_to_completion`.
//!   2. Installs a `MockLlmProvider` that emits turn 1 as a tool_use block
//!      requesting the "Agent" tool, then turn 2 echoes back whatever the
//!      conversation's last `tool_result` content is as an assistant text
//!      block.
//!   3. Calls `Agent::process_message(...)` end-to-end.
//!   4. POSITIVE: the final assistant text contains the sentinel (proves the
//!      real subagent text crossed the seam into turn 2's LLM input).
//!   5. NEGATIVE: the final assistant text does NOT contain "agent_id"
//!      (proves the 104 spawn-marker shape did NOT leak through the seam).
//!
//! Both assertions are required — positive alone doesn't prove the marker
//! is absent, and negative alone doesn't prove the seam carries real data.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use archon_llm::anthropic::AnthropicClient;
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::ToolContext;

use archon_core::agent::{Agent, AgentConfig, AgentEvent, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;

/// Unique sentinel that `run_to_completion` returns. The test proves this
/// string survives the tool_use → tool_result → next-turn LLM input seam
/// by asserting it appears in the final assistant text block.
const SENTINEL: &str = "SENTINEL_104_SEAM_PROOF";

// ---------------------------------------------------------------------------
// FixedStringExecutor — returns the sentinel from run_to_completion.
// ---------------------------------------------------------------------------

struct FixedStringExecutor(&'static str);

#[async_trait]
impl SubagentExecutor for FixedStringExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _req: SubagentRequest,
        _ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        tokio::select! {
            _ = cancel.cancelled() => Err(ExecutorError::Internal("cancelled".into())),
            _ = std::future::ready(()) => Ok(self.0.to_string()),
        }
    }

    async fn on_inner_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
    ) {
    }

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, req: &SubagentRequest) -> SubagentClassification {
        if req.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

// ---------------------------------------------------------------------------
// MockLlmProvider — drives a two-turn conversation.
//
// Turn 1: emit a tool_use block requesting the "Agent" tool with a minimal
//         foreground `{"prompt": "..."}` payload.
// Turn 2: extract the last tool_result's content from the request's
//         `messages` field and emit it as an assistant text block. This
//         echo-through is what lets the test assert end-to-end that the
//         real tool_result content reached the LLM input for turn 2.
//
// Only `stream()` is invoked by `Agent::process_message`; `complete()`,
// `models()`, `supports_feature()` are present to satisfy the trait and
// panic if ever called.
// ---------------------------------------------------------------------------

struct MockLlmProvider {
    turn: Arc<Mutex<u32>>,
}

impl MockLlmProvider {
    fn new() -> Self {
        Self {
            turn: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "mock-model".into(),
            display_name: "Mock".into(),
            context_window: 1_000_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

        let turn = {
            let mut t = self.turn.lock().unwrap();
            *t += 1;
            *t
        };

        // MessageStart — required by the agent loop to initialise usage counters.
        let _ = tx
            .send(StreamEvent::MessageStart {
                id: format!("msg_mock_{turn}"),
                model: "mock-model".into(),
                usage: Usage::default(),
            })
            .await;

        if turn == 1 {
            // Turn 1: a single tool_use block requesting the Agent tool.
            // Foreground: no run_in_background flag, so it hits the
            // SubagentExecutor::run_to_completion path.
            let tool_id = "toolu_mock_1".to_string();
            let _ = tx
                .send(StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some(tool_id.clone()),
                    tool_name: Some("Agent".into()),
                })
                .await;
            let input_json = json!({ "prompt": "run the subagent" }).to_string();
            let _ = tx
                .send(StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: input_json,
                })
                .await;
            let _ = tx.send(StreamEvent::ContentBlockStop { index: 0 }).await;
            let _ = tx
                .send(StreamEvent::MessageDelta {
                    stop_reason: Some("tool_use".into()),
                    usage: None,
                })
                .await;
            let _ = tx.send(StreamEvent::MessageStop).await;
        } else {
            // Turn 2: emit the last tool_result's content as an assistant
            // text block. This echo-through is the whole point — it proves
            // that the seam carried real data from the tool to the LLM.
            let echoed = extract_last_tool_result(&request.messages).unwrap_or_else(|| {
                "<no tool_result found in request.messages>".to_string()
            });
            let _ = tx
                .send(StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                })
                .await;
            let _ = tx
                .send(StreamEvent::TextDelta {
                    index: 0,
                    text: echoed,
                })
                .await;
            let _ = tx.send(StreamEvent::ContentBlockStop { index: 0 }).await;
            let _ = tx
                .send(StreamEvent::MessageDelta {
                    stop_reason: Some("end_turn".into()),
                    usage: None,
                })
                .await;
            let _ = tx.send(StreamEvent::MessageStop).await;
        }

        drop(tx); // close the channel so the agent loop exits its rx.recv() loop
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        panic!("MockLlmProvider::complete should not be called in this test");
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        None
    }
}

/// Extract the content of the most recent `tool_result` block from the
/// conversation's `messages` array (the `user`-role message the agent
/// appends after a tool call).
fn extract_last_tool_result(messages: &[serde_json::Value]) -> Option<String> {
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
            for block in arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(s) = block.get("content").and_then(|c| c.as_str()) {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The regression-guard test.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_process_message_carries_real_subagent_text_across_seam() {
    // 1. Install the FixedStringExecutor FIRST. OnceLock semantics mean the
    //    first installer wins; any later call (including the one
    //    `Agent::install_subagent_executor` would make if we called it) is
    //    a no-op. The test deliberately does NOT call
    //    `agent.install_subagent_executor()`.
    install_subagent_executor(Arc::new(FixedStringExecutor(SENTINEL)));

    // 2. Build a ToolRegistry with only AgentTool registered. The agent
    //    loop's dispatch resolves tools by name from this registry.
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(AgentTool::new()));

    // 3. Build an AgentConfig. Use yolo permission mode so AgentTool
    //    (PermissionLevel::Risky) is auto-allowed without a prompt.
    let mut config = AgentConfig::default();
    config.working_dir = std::env::temp_dir();
    config.session_id = "preserve-104-seam-test".into();
    config.max_turns = Some(5); // safety valve
    *config.permission_mode.lock().await = "yolo".to_string();

    // 4. Build the Agent. Event channel is unbounded; drain it in a
    //    background task so send_event never fails.
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(
        &std::env::temp_dir(),
    )));

    let mut agent = Agent::new(
        Arc::new(MockLlmProvider::new()),
        tools,
        config,
        event_tx,
        agent_registry,
    );

    // 5. Drive the full loop.
    agent
        .process_message("please run a subagent")
        .await
        .expect("process_message failed");

    // 6. Extract the final assistant text block from conversation state.
    let state = agent.conversation_state();
    let final_text = state
        .messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .and_then(|m| m.get("content").and_then(|c| c.as_array()).cloned())
        .and_then(|blocks| {
            blocks.iter().find_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .expect("no final assistant text block found");

    // 7. POSITIVE assertion — the sentinel MUST survive the seam. If the
    //    104 silent-break regression recurs, turn 2's input will be the
    //    JSON spawn marker and the sentinel will be absent.
    assert!(
        final_text.contains(SENTINEL),
        "TASK-AGS-104 regression: final assistant text did not contain the \
         sentinel '{SENTINEL}'. The tool_use → tool_result → next-turn seam \
         dropped the real subagent text. Got: {final_text}"
    );

    // 8. NEGATIVE assertion — the 104 break shape (a spawn marker with
    //    `agent_id` / `status:\"spawned\"`) MUST NOT leak across the seam.
    assert!(
        !final_text.contains("agent_id"),
        "TASK-AGS-104 regression: final assistant text contains 'agent_id', \
         suggesting the foreground path returned a spawn marker instead of \
         the real subagent text. Got: {final_text}"
    );
    assert!(
        !final_text.contains("\"status\":\"spawned\""),
        "TASK-AGS-104 regression: final assistant text contains a \
         '\"status\":\"spawned\"' marker. Got: {final_text}"
    );
}
