//! End-to-end tests for `archon/prompt` against a live agent (issue #26).
//!
//! Everything here runs on a scripted stub provider, so the assertions are
//! about the wiring — which notifications the IDE sees, in what order, what
//! `archon/cancel` does to a stream mid-flight, and whether a tool actually
//! runs — not about any model.

mod ide_support;

use std::sync::Arc;
use std::time::Duration;

use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;

use ide_support::{
    Harness, ScriptedProvider, StallingProvider, message_start, text_block_start, text_delta,
};

#[tokio::test]
async fn prompt_streams_text_deltas_in_order_then_completes() {
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
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
    ]]));
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
    let provider = Arc::new(StallingProvider::new(vec![
        message_start(),
        text_block_start(),
        text_delta("thinking about"),
        text_delta(" your question"),
    ]));
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
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let mut harness = Harness::start(provider);

    let response = harness.cancel();

    assert_eq!(response["result"]["cancelled"], false);
}

#[tokio::test]
async fn a_second_prompt_is_refused_while_a_turn_is_in_flight() {
    let provider = Arc::new(StallingProvider::new(vec![
        message_start(),
        text_block_start(),
        text_delta("working"),
    ]));
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
    let provider = Arc::new(ScriptedProvider::new(vec![]));
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

// ── Protocol-only mode ───────────────────────────────────────────────────────

/// The stub this replaces answered `{"queued": true}` with nothing behind it,
/// which is exactly how `archon serve` came to look like it had accepted a
/// prompt it was never going to run.
#[test]
fn a_handler_with_no_agent_refuses_a_prompt_instead_of_pretending_to_queue_it() {
    let mut handler = archon_sdk::ide::handler::IdeProtocolHandler::new("test");
    let session_id = ide_support::initialize(&mut handler, true);

    let response: serde_json::Value = serde_json::from_str(
        &handler.handle(
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "archon/prompt",
                "params": {"sessionId": session_id, "text": "hi"},
            })
            .to_string(),
        ),
    )
    .expect("valid JSON");

    assert!(response.get("result").is_none(), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("no agent is attached")),
        "{response}"
    );
}

// ── archon/status ────────────────────────────────────────────────────────────

#[tokio::test]
async fn status_reports_an_absence_before_the_first_turn_rather_than_zeros() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let mut harness = Harness::start(provider);

    let status = harness.status();

    assert!(
        status["result"].get("inputTokens").is_none(),
        "a session that has run nothing has no token reading: {status}"
    );
    assert!(
        status["result"]["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("no turn has completed")),
        "{status}"
    );
    assert!(status["result"]["model"].as_str().is_some(), "{status}");
}

#[tokio::test]
async fn status_reports_measured_tokens_once_a_turn_has_run() {
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
        message_start(),
        text_block_start(),
        text_delta("done"),
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
    ]]));
    let mut harness = Harness::start(provider);

    // Not a greeting: `try_complete_trivial_cognitive_turn` answers those
    // without calling the provider at all, so the turn would legitimately
    // report no tokens and the assertion below would be testing nothing.
    harness.prompt("walk me through the module I have open");
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;

    let status = harness.status();

    assert!(
        status["result"].get("unavailable").is_none(),
        "a completed turn is a measurement: {status}"
    );
    assert_eq!(status["result"]["inputTokens"], 11, "{status}");
    assert_eq!(status["result"]["outputTokens"], 7, "{status}");
}

// ── archon/toolResult and archon/config ──────────────────────────────────────

#[tokio::test]
async fn tool_result_is_refused_rather_than_acknowledged() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let mut harness = Harness::start(provider);

    let response = harness.request(
        "archon/toolResult",
        serde_json::json!({
            "sessionId": harness.session_id,
            "toolUseId": "tool-1",
            "result": "42",
            "isError": false,
        }),
    );

    assert!(response.get("result").is_none(), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("not supported")),
        "{response}"
    );
}

#[tokio::test]
async fn config_round_trips_the_permission_mode_the_gate_actually_reads() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let mut harness = Harness::start(provider);

    let written = harness.request(
        "archon/config",
        serde_json::json!({"key": "permissionMode", "value": "plan"}),
    );
    assert_eq!(written["result"]["ok"], true, "{written}");

    let read = harness.request(
        "archon/config",
        serde_json::json!({"key": "permissionMode"}),
    );
    assert_eq!(read["result"]["value"], "plan", "{read}");

    // The write landed on the agent's own handle, not a copy beside it.
    let live = harness.agent.lock().await.permission_mode_handle();
    assert_eq!(*live.lock().await, "plan");
}

#[tokio::test]
async fn config_refuses_a_key_it_does_not_know() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let mut harness = Harness::start(provider);

    let response = harness.request("archon/config", serde_json::json!({"key": "permisionMode"}));

    assert!(response.get("result").is_none(), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("unknown config key")),
        "{response}"
    );
}
