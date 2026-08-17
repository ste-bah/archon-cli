//! #187: lifecycle-hook context must actually reach the model.
//!
//! `AggregatedHookResult.additional_contexts` was aggregated by the registry
//! for every event but read by exactly one caller — `PostToolUse`. A hook that
//! contributed context at `SessionStart` or `PostCompact` ran, produced correct
//! output, and changed nothing about what the model saw.
//!
//! That class of bug is invisible to a unit test on the setter: the field can
//! be populated correctly and still never be rendered into a request. So this
//! drives `Agent::process_message` end to end against a provider that captures
//! the `system` blocks it is handed, and asserts the context is in them.
//!
//! The negative half matters as much as the positive: an agent with no hook
//! contribution must not emit an empty `<hook-context>` block, or every session
//! pays tokens for a wrapper around nothing.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;

use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;

const HOOK_TEXT: &str = "HOOK_CONTEXT_187_PROOF";

/// Input substantive enough to reach the provider.
///
/// Two things make this fiddly, and both are worth naming:
///
/// 1. `process_message` short-circuits through
///    `try_complete_trivial_cognitive_turn` before building a request, so a
///    greeting is answered without ever calling the LLM. A test asserting "no
///    block present" would then pass while proving nothing — hence the
///    request-count guard before the assertions.
/// 2. The classifier keys off the input text, and turn handling is memoised,
///    so reusing one string across turns can skip the provider entirely. Each
///    turn therefore passes a distinct discriminator.
fn substantive_input(discriminator: &str) -> String {
    format!(
        "Refactor the token budget calculation in the {discriminator} assembler \
         and explain the tradeoffs between the two approaches."
    )
}

/// Captures the `system` blocks of every request, then ends the turn with a
/// one-line assistant message so the agent loop terminates immediately.
struct CapturingProvider {
    seen_system: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "mock-model".into(),
            display_name: "Mock".into(),
            context_window: 1_000_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        // One entry per request, so a test can compare turns rather than
        // inspecting a flattened blob of all of them.
        let flattened = request
            .system
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        self.seen_system.lock().unwrap().push(flattened);

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
        let _ = tx
            .send(StreamEvent::MessageStart {
                id: "msg_capture".into(),
                model: "mock-model".into(),
                usage: Usage::default(),
            })
            .await;
        let _ = tx
            .send(StreamEvent::ContentBlockStart {
                index: 0,
                block_type: archon_llm::types::ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            })
            .await;
        // agent-event-tx-lint: ignore — channel holds StreamEvent, not AgentEvent
        let _ = tx
            .send(StreamEvent::TextDelta {
                index: 0,
                text: "done".into(),
            })
            .await;
        // agent-event-tx-lint: ignore — channel holds StreamEvent, not AgentEvent
        let _ = tx.send(StreamEvent::ContentBlockStop { index: 0 }).await;
        let _ = tx
            .send(StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            })
            .await;
        let _ = tx.send(StreamEvent::MessageStop).await;
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("the agent loop only streams")
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

/// Build an agent whose provider records the system blocks it receives.
async fn agent_with_capture() -> (Agent, Arc<Mutex<Vec<String>>>) {
    let seen_system = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(CapturingProvider {
        seen_system: Arc::clone(&seen_system),
    });

    let config = AgentConfig {
        working_dir: std::env::temp_dir(),
        session_id: "hook-context-187".into(),
        max_turns: Some(2),
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = "yolo".to_string();

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<TimestampedEvent>(
        archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY,
    );
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(
        &std::env::temp_dir(),
    )));

    let agent = Agent::new(
        provider,
        ToolRegistry::new(),
        config,
        event_tx,
        agent_registry,
    );
    (agent, seen_system)
}

/// The wire that was cut, tested as a transition on one agent.
///
/// Both halves live in one test on purpose. Asserting "no block present" in a
/// separate test is worthless if the provider was never called, and the
/// short-circuit above makes that easy to do by accident. Comparing turn 1
/// against turn 2 on the same agent means the negative can only pass if the
/// positive did.
#[tokio::test]
async fn hook_contributed_context_reaches_the_model() {
    let (mut agent, seen_system) = agent_with_capture().await;

    // Turn 1: nothing contributed yet.
    agent
        .process_message(&substantive_input("context"))
        .await
        .expect("first turn failed");

    // Turn 2: a hook has since contributed.
    agent.add_hook_session_context(vec![HOOK_TEXT.to_string()]);
    agent
        .process_message(&substantive_input("scheduling"))
        .await
        .expect("second turn failed");

    let requests = seen_system.lock().unwrap().clone();
    assert!(
        requests.len() >= 2,
        "expected a request per turn, got {}: the turn short-circuited and \
         this test would prove nothing",
        requests.len()
    );

    let first = requests.first().expect("first request");
    let last = requests.last().expect("last request");

    assert!(
        !first.contains("<hook-context>"),
        "no contribution must emit no block — an empty wrapper costs every \
         session tokens for nothing: {first}"
    );
    assert!(
        last.contains(HOOK_TEXT),
        "hook context never reached the request: {last}"
    );
    assert!(
        last.contains("<hook-context>"),
        "context should be delimited so the model can tell it apart: {last}"
    );
}

/// Several hooks fire per event, and `PostCompact` adds to what `SessionStart`
/// established. Accumulation is the required behaviour; replacement would drop
/// the session-start bootstrap at the first compaction.
#[tokio::test]
async fn contributions_accumulate_across_events() {
    let (mut agent, _) = agent_with_capture().await;

    agent.add_hook_session_context(vec!["from-session-start".to_string()]);
    agent.add_hook_session_context(vec!["from-post-compact".to_string()]);

    assert_eq!(
        agent.hook_session_context(),
        ["from-session-start", "from-post-compact"]
    );
}

/// A hook that returns whitespace must not leave a stray line in the prompt.
#[tokio::test]
async fn blank_contributions_are_dropped() {
    let (mut agent, _) = agent_with_capture().await;

    agent.add_hook_session_context(vec!["   ".to_string(), String::new(), "real".to_string()]);

    assert_eq!(agent.hook_session_context(), ["real"]);
}
