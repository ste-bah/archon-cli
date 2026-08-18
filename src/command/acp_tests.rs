//! Tests for the ACP agent adapter (#189 Phase 11).
//!
//! The protocol itself is tested in `archon-acp` against a stub. What is left
//! here is the translation: which agent events an editor sees, and what it is
//! told about them.

use super::*;

use archon_acp::jsonrpc::Incoming;
use archon_tools::tool::ToolResult;

/// Capture what one event turns into on the wire.
fn forwarded(event: AgentEvent) -> Vec<serde_json::Value> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let peer = Peer::new(tx);

    forward(&peer, "sess_1", event);

    let mut sent = Vec::new();
    while let Ok(line) = rx.try_recv() {
        sent.push(serde_json::from_str(&line).expect("what is sent is JSON"));
    }
    sent
}

fn update_of(event: AgentEvent) -> Option<serde_json::Value> {
    forwarded(event)
        .into_iter()
        .next()
        .map(|value| value["params"]["update"].clone())
}

#[test]
fn streamed_text_becomes_an_agent_message_chunk() {
    let update = update_of(AgentEvent::TextDelta("hello".into())).expect("forwarded");

    assert_eq!(update["sessionUpdate"], "agent_message_chunk");
    assert_eq!(update["content"]["text"], "hello");
}

#[test]
fn thinking_becomes_a_thought_chunk_so_a_client_can_fold_it_away() {
    let update = update_of(AgentEvent::ThinkingDelta("reasoning".into())).expect("forwarded");

    assert_eq!(update["sessionUpdate"], "agent_thought_chunk");
}

/// The preview exists to be revised, and an editor has nowhere to un-render it.
#[test]
fn a_transient_thinking_preview_is_not_forwarded() {
    assert!(
        forwarded(AgentEvent::TransientThinkingDelta("draft".into())).is_empty(),
        "an unapproved preview must not reach the client"
    );
}

#[test]
fn a_started_tool_call_names_itself_and_reports_its_kind() {
    let update = update_of(AgentEvent::ToolCallStarted {
        name: "Bash".into(),
        id: "call_7".into(),
    })
    .expect("forwarded");

    assert_eq!(update["sessionUpdate"], "tool_call");
    assert_eq!(update["toolCallId"], "call_7");
    assert_eq!(update["title"], "Bash");
    assert_eq!(update["status"], "in_progress");
    assert_eq!(
        update["kind"], "execute",
        "a shell command shown as a read would understate it"
    );
}

#[test]
fn a_completed_tool_call_carries_its_output() {
    let update = update_of(AgentEvent::ToolCallComplete {
        name: "Read".into(),
        id: "call_7".into(),
        result: ToolResult::success("file contents"),
        transcript_summary: None,
    })
    .expect("forwarded");

    assert_eq!(update["sessionUpdate"], "tool_call_update");
    assert_eq!(update["status"], "completed");
    assert_eq!(update["content"][0]["content"]["text"], "file contents");
}

/// A failed tool must not be reported as completed: the editor renders the two
/// differently, and a user reading "completed" over an error is being misled.
#[test]
fn a_failed_tool_call_is_reported_as_failed() {
    let update = update_of(AgentEvent::ToolCallComplete {
        name: "Read".into(),
        id: "call_7".into(),
        result: ToolResult::error("no such file"),
        transcript_summary: None,
    })
    .expect("forwarded");

    assert_eq!(update["status"], "failed", "{update}");
    assert!(
        update["content"][0]["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("no such file"),
        "{update}"
    );
}

/// An editor renders this inline beside the call, so a whole build log would
/// bury the conversation. The agent still has the full output in its context.
#[test]
fn an_oversized_tool_result_is_truncated_and_says_so() {
    let update = update_of(AgentEvent::ToolCallComplete {
        name: "Bash".into(),
        id: "call_7".into(),
        result: ToolResult::success("x".repeat(MAX_TOOL_RESULT_CHARS + 500)),
        transcript_summary: None,
    })
    .expect("forwarded");

    let text = update["content"][0]["content"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("[truncated for display]"), "not marked");
    assert!(text.chars().count() < MAX_TOOL_RESULT_CHARS + 100);
}

#[test]
fn a_result_at_the_limit_is_not_truncated() {
    let exact = "y".repeat(MAX_TOOL_RESULT_CHARS);

    assert_eq!(truncate(&exact), exact);
}

/// Telemetry has no ACP counterpart, and forcing it into assistant prose would
/// be worse than not showing it.
#[test]
fn events_with_no_counterpart_are_dropped_rather_than_shown_as_prose() {
    assert!(
        forwarded(AgentEvent::ApiCallStarted {
            model: "claude-sonnet-4-6".into()
        })
        .is_empty()
    );
    assert!(forwarded(AgentEvent::UserPromptReady).is_empty());
}

/// Everything forwarded is addressed to the session it came from.
#[test]
fn every_update_names_its_session() {
    let sent = forwarded(AgentEvent::TextDelta("hi".into()));

    assert_eq!(sent[0]["method"], "session/update");
    assert_eq!(sent[0]["params"]["sessionId"], "sess_1");
    let parsed: Incoming = serde_json::from_value(sent[0].clone()).expect("parses as a message");
    assert!(
        parsed.is_notification(),
        "an update must be a notification, not a request"
    );
}
