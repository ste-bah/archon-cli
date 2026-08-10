//! Shared harness for the IDE integration tests (issue #26).
//!
//! Deterministic by construction: the LLM is a scripted queue of stream
//! events, and the only tool is a probe that records whether it ran. Nothing
//! here reaches a model, a network, or the real tool registry.
//!
//! Compiled separately into each IDE test binary, so anything only one of them
//! uses looks dead to the others.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};

use archon_core::agent::{AGENT_EVENT_CHANNEL_CAPACITY, Agent, AgentConfig};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_sdk::ide::handler::IdeProtocolHandler;
use archon_sdk::ide::protocol::JRpcNotification;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ── Stub providers ───────────────────────────────────────────────────────────

/// Replays one scripted round per `stream` call, then closes the stream.
///
/// A queue rather than a single script because a turn that calls a tool needs
/// two rounds: the one that asks for the tool, and the one that answers with
/// the result in hand.
pub struct ScriptedProvider {
    rounds: std::sync::Mutex<std::collections::VecDeque<Vec<StreamEvent>>>,
}

/// Streams a prelude and then stalls forever, so a turn can be caught
/// mid-stream and cancelled.
pub struct StallingProvider {
    prelude: std::sync::Mutex<Vec<StreamEvent>>,
}

impl ScriptedProvider {
    pub fn new(rounds: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            rounds: std::sync::Mutex::new(rounds.into()),
        }
    }
}

impl StallingProvider {
    pub fn new(prelude: Vec<StreamEvent>) -> Self {
        Self {
            prelude: std::sync::Mutex::new(prelude),
        }
    }
}

pub fn message_start() -> StreamEvent {
    StreamEvent::MessageStart {
        id: "msg-ide".into(),
        model: "stub".into(),
        usage: Usage {
            input_tokens: 11,
            output_tokens: 0,
            ..Usage::default()
        },
    }
}

pub fn text_block_start() -> StreamEvent {
    StreamEvent::ContentBlockStart {
        index: 0,
        block_type: ContentBlockType::Text,
        tool_use_id: None,
        tool_name: None,
    }
}

pub fn text_delta(text: &str) -> StreamEvent {
    StreamEvent::TextDelta {
        index: 0,
        text: text.into(),
    }
}

/// One round that asks to run `tool` and nothing else.
pub fn tool_use_round(tool: &str, tool_use_id: &str) -> Vec<StreamEvent> {
    vec![
        message_start(),
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some(tool_use_id.into()),
            tool_name: Some(tool.into()),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: "{}".into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("tool_use".into()),
            usage: Some(Usage {
                input_tokens: 11,
                output_tokens: 3,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]
}

/// One round that replies with `text` and ends the turn.
pub fn text_round(text: &str) -> Vec<StreamEvent> {
    vec![
        message_start(),
        text_block_start(),
        text_delta(text),
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                input_tokens: 11,
                output_tokens: 7,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let events = self
            .rounds
            .lock()
            .expect("script lock")
            .pop_front()
            .unwrap_or_default();
        let (tx, rx) = mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        // Dropping `tx` here closes the stream, which is how the agent loop
        // learns the round is over.
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("the IDE surface only streams")
    }
}

#[async_trait::async_trait]
impl LlmProvider for StallingProvider {
    fn name(&self) -> &str {
        "stalling"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let prelude: Vec<StreamEvent> = self
            .prelude
            .lock()
            .expect("prelude lock")
            .drain(..)
            .collect();
        let (tx, rx) = mpsc::channel(prelude.len() + 1);
        tokio::spawn(async move {
            for event in prelude {
                if tx.send(event).await.is_err() {
                    return;
                }
            }
            // Hold the sender open so the turn never finishes on its own.
            // Resolves when the agent drops the receiver, which is exactly
            // what cancellation does — so this task cannot outlive the test.
            tx.closed().await;
        });
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("the IDE surface only streams")
    }
}

// ── Probe tool ───────────────────────────────────────────────────────────────

/// Name chosen so it is absent from `DEFAULT_SAFE_TOOLS`; a safe tool would be
/// auto-allowed and never reach the permission gate at all.
pub const PROBE_TOOL: &str = "ProbeWrite";

/// A tool whose entire job is to record that it was allowed to run.
///
/// This is what makes "denied" testable as a fact rather than as a log line:
/// if the flag is set, the tool executed, whatever the notifications said.
pub struct ProbeTool {
    executed: Arc<AtomicBool>,
}

impl ProbeTool {
    pub fn new() -> (Self, Arc<AtomicBool>) {
        let executed = Arc::new(AtomicBool::new(false));
        (
            Self {
                executed: Arc::clone(&executed),
            },
            executed,
        )
    }
}

#[async_trait::async_trait]
impl Tool for ProbeTool {
    fn name(&self) -> &str {
        PROBE_TOOL
    }

    fn description(&self) -> &str {
        "Records that it ran. Test double for a tool with real side effects."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executed.store(true, Ordering::SeqCst);
        ToolResult::success("probe ran")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Dangerous
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

pub struct Harness {
    pub handler: IdeProtocolHandler,
    pub notifications: mpsc::Receiver<JRpcNotification>,
    pub agent: Arc<Mutex<Agent>>,
    pub session_id: String,
    next_id: u64,
}

impl Harness {
    /// Build a handler wired to a live agent with no tools, and complete the
    /// handshake as a client that can approve tool use.
    pub fn start(provider: Arc<dyn LlmProvider>) -> Self {
        Self::build(provider, ToolRegistry::new(), "auto", true)
    }

    /// Build a handler wired to a live agent that owns `tools`, running in
    /// `permission_mode`, for a client that advertises `tool_execution` as
    /// `client_can_approve`.
    pub fn with_tools(
        provider: Arc<dyn LlmProvider>,
        tools: ToolRegistry,
        permission_mode: &str,
        client_can_approve: bool,
    ) -> Self {
        Self::build(provider, tools, permission_mode, client_can_approve)
    }

    fn build(
        provider: Arc<dyn LlmProvider>,
        tools: ToolRegistry,
        permission_mode: &str,
        client_can_approve: bool,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
        let config = AgentConfig {
            permission_mode: Arc::new(Mutex::new(permission_mode.to_string())),
            ..AgentConfig::default()
        };
        let agent = Agent::new(
            provider,
            tools,
            config,
            event_tx,
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(
                &std::env::temp_dir(),
            ))),
        );
        let (mut handler, notifications, agent) =
            IdeProtocolHandler::with_agent("test", agent, event_rx);
        let session_id = initialize(&mut handler, client_can_approve);

        Self {
            handler,
            notifications,
            agent,
            session_id,
            next_id: 100,
        }
    }

    pub fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        serde_json::from_str(&self.handler.handle(&raw)).expect("response is valid JSON")
    }

    pub fn prompt(&mut self, text: &str) -> serde_json::Value {
        self.request(
            "archon/prompt",
            serde_json::json!({"sessionId": self.session_id, "text": text}),
        )
    }

    pub fn cancel(&mut self) -> serde_json::Value {
        self.request(
            "archon/cancel",
            serde_json::json!({"sessionId": self.session_id}),
        )
    }

    pub fn status(&mut self) -> serde_json::Value {
        self.request(
            "archon/status",
            serde_json::json!({"sessionId": self.session_id}),
        )
    }

    pub fn answer_permission(&mut self, request_id: &str, approved: bool) -> serde_json::Value {
        self.request(
            "archon/permissionResponse",
            serde_json::json!({
                "sessionId": self.session_id,
                "requestId": request_id,
                "approved": approved,
            }),
        )
    }

    /// Next notification, or a test failure if the stream goes quiet.
    pub async fn next_notification(&mut self) -> JRpcNotification {
        tokio::time::timeout(Duration::from_secs(10), self.notifications.recv())
            .await
            .expect("timed out waiting for a notification")
            .expect("notification channel closed early")
    }

    /// Read notifications until one with `method` arrives, and return it.
    pub async fn drain_until(&mut self, method: &str) -> JRpcNotification {
        loop {
            let notification = self.next_notification().await;
            if notification.method == method {
                return notification;
            }
        }
    }

    /// Assert nothing is emitted for `window`, and return the silence.
    pub async fn expect_silence(&mut self, window: Duration) {
        if let Ok(unexpected) = tokio::time::timeout(window, self.notifications.recv()).await {
            panic!("expected the agent to be waiting, got {unexpected:?}");
        }
    }

    /// Wait until nothing holds the agent lock — i.e. the turn task has
    /// actually let go, rather than merely having been asked to.
    pub async fn wait_for_idle_agent(&self) {
        for _ in 0..500 {
            if self.agent.try_lock().is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("turn never released the agent");
    }
}

/// Complete `archon/initialize` and return the negotiated session id.
pub fn initialize(handler: &mut IdeProtocolHandler, tool_execution: bool) -> String {
    let raw = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "archon/initialize",
        "params": {
            "clientInfo": {"name": "test", "version": "1.0"},
            "capabilities": {
                "inlineCompletion": false,
                "toolExecution": tool_execution,
                "diff": false,
                "terminal": false,
            },
        },
    })
    .to_string();
    let response: serde_json::Value =
        serde_json::from_str(&handler.handle(&raw)).expect("response is valid JSON");
    response["result"]["sessionId"]
        .as_str()
        .expect("initialize returns a sessionId")
        .to_string()
}
