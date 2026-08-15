use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::{MessageHistory, collect_stream_round, compact_messages_for_retry, projected_request};
use crate::subagent::runner::SubagentRunner;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_tools::tool::ToolContext;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

include!("stream_round_test_fixture.rs");
include!("stream_round_recovery_tests.rs");

#[tokio::test]
async fn cancellation_drops_stalled_provider_stream_promptly() {
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let cancel = CancellationToken::new();
    let stalled = SubagentRunner::new(
        Arc::new(StalledProvider {
            started: Mutex::new(Some(started_tx)),
            dropped: Mutex::new(Some(dropped_tx)),
        }),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext {
            cancel_parent: Some(cancel.clone()),
            ..ToolContext::default()
        },
        "stalled-model".into(),
        1,
        60,
        Arc::new(crate::agent::AgentConfig::default()),
        Arc::new(test_identity()),
    );
    let run = tokio::spawn(async move { stalled.run("wait forever").await });
    tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
        .await
        .expect("provider stream should start")
        .expect("start signal should be sent");

    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("runner should observe cancellation")
        .expect("runner task should not panic")
        .expect_err("cancelled inference should not succeed");
    assert!(
        result
            .to_string()
            .contains("Subagent cancelled during LLM inference")
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("provider receiver should be dropped")
        .expect("drop signal should be sent");
}

#[test]
fn rebuilt_request_marks_latest_text_without_mutating_history() {
    let runner = runner();
    let messages = vec![serde_json::json!({"role":"user","content":"rebuilt text"})];

    let projected = projected_request(&runner, &messages, &LlmRequest::default());

    assert_eq!(
        projected.messages[0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(messages[0]["content"], "rebuilt text");
}

#[test]
fn rebuilt_request_marks_latest_tool_result_without_mutating_history() {
    let runner = runner();
    let messages = vec![
        serde_json::json!({"role":"user","content":"initial"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"tool-1","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"tool-1","content":"result"
        }]}),
    ];

    let projected = projected_request(&runner, &messages, &LlmRequest::default());

    assert_eq!(
        projected.messages[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert!(
        projected.messages[0]["content"]
            .as_str()
            .is_some_and(|text| text == "initial")
    );
    assert!(messages[2]["content"][0].get("cache_control").is_none());
}

/// The idle guard is a stalled-provider check, not a thinking budget. It was
/// hardcoded at 120s — sixty times tighter than the host_call_timeout_secs
/// stage it runs inside — and killed a live inventory reducer three turns in.
#[test]
fn the_idle_timeout_comes_from_configuration() {
    let mut agent_config = crate::agent::AgentConfig {
        subagent_stream_idle_timeout_secs: 900,
        ..Default::default()
    };
    let runner = |config: &crate::agent::AgentConfig| {
        SubagentRunner::new(
            Arc::new(StalledProvider {
                started: Mutex::new(None),
                dropped: Mutex::new(None),
            }),
            String::new(),
            Vec::new(),
            Arc::new(crate::dispatch::ToolRegistry::new()),
            ToolContext::default(),
            "model".into(),
            1,
            60,
            Arc::new(config.clone()),
            Arc::new(test_identity()),
        )
    };

    assert_eq!(
        super::stream_idle_timeout(&runner(&agent_config)),
        std::time::Duration::from_secs(900)
    );

    // A zero would make every round fail instantly, so it is floored at one.
    agent_config.subagent_stream_idle_timeout_secs = 0;
    assert_eq!(
        super::stream_idle_timeout(&runner(&agent_config)),
        std::time::Duration::from_secs(1)
    );
}

/// The shipped default must be generous enough that only a genuinely stalled
/// provider trips it, and still well inside the enclosing stage timeout.
#[test]
fn the_default_idle_timeout_is_not_a_thinking_budget() {
    assert_eq!(
        crate::agent::AgentConfig::default().subagent_stream_idle_timeout_secs,
        crate::config::DEFAULT_STREAM_IDLE_TIMEOUT_SECS
    );
    assert!(crate::config::DEFAULT_STREAM_IDLE_TIMEOUT_SECS >= 600);
}
