//! End-to-end tests for `archon/prompt` against a live agent (issue #26).
//!
//! Everything here runs on a scripted stub provider, so the assertions are
//! about the wiring — which notifications the IDE sees, in what order, and
//! what `archon/cancel` does to a stream mid-flight — not about any model.

use std::sync::Arc;
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

// ── Stub providers ───────────────────────────────────────────────────────────

/// Replays one scripted turn and then closes the stream.
struct ScriptedProvider {
    events: std::sync::Mutex<Vec<StreamEvent>>,
}

/// Streams a prelude and then stalls forever, so a turn can be caught
/// mid-stream and cancelled.
struct StallingProvider {
    prelude: std::sync::Mutex<Vec<StreamEvent>>,
}

fn message_start() -> StreamEvent {
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

fn text_block_start() -> StreamEvent {
    StreamEvent::ContentBlockStart {
        index: 0,
        block_type: ContentBlockType::Text,
        tool_use_id: None,
        tool_name: None,
    }
}

fn text_delta(text: &str) -> StreamEvent {
    StreamEvent::TextDelta {
        index: 0,
        text: text.into(),
    }
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
        let events: Vec<StreamEvent> = self.events.lock().expect("script lock").drain(..).collect();
        let (tx, rx) = mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        // Dropping `tx` here closes the stream, which is how the agent loop
        // learns the round is over.
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("IDE slice 1 only streams")
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
        unimplemented!("IDE slice 1 only streams")
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

struct Harness {
    handler: IdeProtocolHandler,
    notifications: mpsc::Receiver<JRpcNotification>,
    agent: Arc<Mutex<Agent>>,
    session_id: String,
    next_id: u64,
}

impl Harness {
    /// Build a handler wired to a live agent and complete the handshake.
    ///
    /// The tool registry is empty on purpose: this slice has no permission
    /// round-trip, and an agent with `permission_response_rx == None`
    /// auto-approves everything it is asked to run.
    fn start(provider: Arc<dyn LlmProvider>) -> Self {
        let (event_tx, event_rx) = mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
        let agent = Arc::new(Mutex::new(Agent::new(
            provider,
            ToolRegistry::new(),
            AgentConfig::default(),
            event_tx,
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(
                &std::env::temp_dir(),
            ))),
        )));
        let (handler, notifications) =
            IdeProtocolHandler::with_agent("test", Arc::clone(&agent), event_rx);

        let mut harness = Self {
            handler,
            notifications,
            agent,
            session_id: String::new(),
            next_id: 1,
        };
        harness.session_id = harness.initialize();
        harness
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
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

    fn initialize(&mut self) -> String {
        let response = self.request(
            "archon/initialize",
            serde_json::json!({
                "clientInfo": {"name": "test", "version": "1.0"},
                "capabilities": {
                    "inlineCompletion": false,
                    "toolExecution": false,
                    "diff": false,
                    "terminal": false,
                },
            }),
        );
        response["result"]["sessionId"]
            .as_str()
            .expect("initialize returns a sessionId")
            .to_string()
    }

    fn prompt(&mut self, text: &str) -> serde_json::Value {
        self.request(
            "archon/prompt",
            serde_json::json!({"sessionId": self.session_id, "text": text}),
        )
    }

    fn cancel(&mut self) -> serde_json::Value {
        self.request(
            "archon/cancel",
            serde_json::json!({"sessionId": self.session_id}),
        )
    }

    /// Next notification, or a test failure if the stream goes quiet.
    async fn next_notification(&mut self) -> JRpcNotification {
        tokio::time::timeout(Duration::from_secs(10), self.notifications.recv())
            .await
            .expect("timed out waiting for a notification")
            .expect("notification channel closed early")
    }

    /// Wait until nothing holds the agent lock — i.e. the turn task has
    /// actually let go, rather than merely having been asked to.
    async fn wait_for_idle_agent(&self) {
        for _ in 0..500 {
            if self.agent.try_lock().is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("cancelled turn never released the agent");
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn prompt_streams_text_deltas_in_order_then_completes() {
    let provider = Arc::new(ScriptedProvider {
        events: std::sync::Mutex::new(vec![
            message_start(),
            text_block_start(),
            text_delta("Hello"),
            text_delta(", world"),
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
        ]),
    });
    let mut harness = Harness::start(provider);

    let response = harness.prompt("summarise the module I have open");
    assert_eq!(
        response["result"]["queued"], true,
        "prompt was not accepted"
    );

    let mut deltas = Vec::new();
    let complete = loop {
        let notification = harness.next_notification().await;
        match notification.method.as_str() {
            "archon/textDelta" => deltas.push(
                notification.params["text"]
                    .as_str()
                    .expect("textDelta carries text")
                    .to_string(),
            ),
            "archon/turnComplete" => break notification,
            other => panic!("unexpected notification during a text-only turn: {other}"),
        }
    };

    assert_eq!(deltas, vec!["Hello".to_string(), ", world".to_string()]);
    assert_eq!(
        complete.params["sessionId"], harness.session_id,
        "notifications must carry the negotiated sessionId, not a fresh one"
    );
}

#[tokio::test]
async fn cancel_stops_an_in_flight_stream() {
    let provider = Arc::new(StallingProvider {
        prelude: std::sync::Mutex::new(vec![
            message_start(),
            text_block_start(),
            text_delta("thinking about"),
            text_delta(" your question"),
        ]),
    });
    let mut harness = Harness::start(provider);

    harness.prompt("walk me through this file");
    for expected in ["thinking about", " your question"] {
        let notification = harness.next_notification().await;
        assert_eq!(notification.method, "archon/textDelta");
        assert_eq!(notification.params["text"], expected);
    }

    let response = harness.cancel();
    assert_eq!(
        response["result"]["cancelled"], true,
        "cancel must report that a turn was running"
    );

    // The turn is genuinely torn down, not merely flagged: the lock is free
    // again and no completion is ever announced for the abandoned turn.
    harness.wait_for_idle_agent().await;
    let trailing =
        tokio::time::timeout(Duration::from_millis(250), harness.notifications.recv()).await;
    assert!(
        trailing.is_err(),
        "cancelled turn kept emitting: {trailing:?}"
    );
}

#[tokio::test]
async fn cancelling_an_idle_session_reports_nothing_to_cancel() {
    let provider = Arc::new(ScriptedProvider {
        events: std::sync::Mutex::new(Vec::new()),
    });
    let mut harness = Harness::start(provider);

    let response = harness.cancel();

    assert_eq!(response["result"]["cancelled"], false);
}

#[tokio::test]
async fn a_second_prompt_is_refused_while_a_turn_is_in_flight() {
    let provider = Arc::new(StallingProvider {
        prelude: std::sync::Mutex::new(vec![
            message_start(),
            text_block_start(),
            text_delta("working"),
        ]),
    });
    let mut harness = Harness::start(provider);

    harness.prompt("first question");
    let first_delta = harness.next_notification().await;
    assert_eq!(first_delta.method, "archon/textDelta");

    // Two concurrent turns would interleave their deltas on one stream and
    // the IDE has no way to tell them apart, so the second is rejected.
    let response = harness.prompt("second question");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("already in flight")),
        "expected an in-flight rejection, got {response}"
    );

    harness.cancel();
    harness.wait_for_idle_agent().await;
}

#[tokio::test]
async fn prompt_for_an_unknown_session_never_reaches_the_agent() {
    let provider = Arc::new(ScriptedProvider {
        events: std::sync::Mutex::new(Vec::new()),
    });
    let mut harness = Harness::start(provider);

    let response = harness.request(
        "archon/prompt",
        serde_json::json!({"sessionId": "not-a-session", "text": "hi"}),
    );

    assert_eq!(response["error"]["code"], -32602);
    assert!(
        harness.agent.try_lock().is_ok(),
        "no turn should have begun"
    );
}
