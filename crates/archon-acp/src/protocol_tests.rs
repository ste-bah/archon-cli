//! Wire-format tests (#189 Phase 11).
//!
//! These assert on JSON, not on Rust types. A round-trip test would pass with
//! every field renamed, because both halves would be renamed together — and the
//! result would be an agent no editor can talk to. The spellings below are the
//! protocol's; they are pinned here so a refactor has to notice.

use super::*;

fn json(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_value(value).expect("serialises")
}

#[test]
fn an_agent_message_chunk_is_tagged_on_session_update() {
    let update = SessionUpdate::AgentMessageChunk {
        content: ContentBlock::text("hello"),
    };

    assert_eq!(
        json(&update),
        serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" }
        })
    );
}

#[test]
fn a_thought_chunk_has_its_own_discriminator() {
    let update = SessionUpdate::AgentThoughtChunk {
        content: ContentBlock::text("thinking"),
    };

    assert_eq!(json(&update)["sessionUpdate"], "agent_thought_chunk");
}

#[test]
fn a_tool_call_carries_the_camel_case_id_title_kind_and_status() {
    let update = SessionUpdate::ToolCall {
        tool_call_id: "call_001".into(),
        title: "Read README.md".into(),
        kind: ToolKind::Read,
        status: ToolStatus::Pending,
    };

    assert_eq!(
        json(&update),
        serde_json::json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_001",
            "title": "Read README.md",
            "kind": "read",
            "status": "pending"
        })
    );
}

#[test]
fn a_tool_call_update_omits_empty_content_rather_than_sending_null() {
    let update = SessionUpdate::ToolCallUpdate {
        tool_call_id: "call_001".into(),
        status: ToolStatus::Completed,
        content: Vec::new(),
    };

    let value = json(&update);
    assert_eq!(value["status"], "completed");
    assert!(
        value.get("content").is_none(),
        "an absent result is not an empty one: {value}"
    );
}

#[test]
fn tool_call_content_is_double_wrapped_the_way_the_protocol_asks() {
    let update = SessionUpdate::ToolCallUpdate {
        tool_call_id: "call_001".into(),
        status: ToolStatus::Completed,
        content: vec![ToolCallContent::text("done")],
    };

    assert_eq!(
        json(&update)["content"],
        serde_json::json!([{ "type": "content", "content": { "type": "text", "text": "done" } }])
    );
}

#[test]
fn a_session_notification_wraps_the_update_under_session_id() {
    let notification = SessionNotification {
        session_id: "sess_1".into(),
        update: SessionUpdate::AgentMessageChunk {
            content: ContentBlock::text("hi"),
        },
    };

    let value = json(&notification);
    assert_eq!(value["sessionId"], "sess_1");
    assert_eq!(value["update"]["sessionUpdate"], "agent_message_chunk");
}

#[test]
fn every_stop_reason_uses_the_protocols_spelling() {
    for (reason, expected) in [
        (StopReason::EndTurn, "end_turn"),
        (StopReason::MaxTokens, "max_tokens"),
        (StopReason::MaxTurnRequests, "max_turn_requests"),
        (StopReason::Refusal, "refusal"),
        (StopReason::Cancelled, "cancelled"),
    ] {
        assert_eq!(json(&reason), expected);
    }
}

#[test]
fn every_tool_status_uses_the_protocols_spelling() {
    for (status, expected) in [
        (ToolStatus::Pending, "pending"),
        (ToolStatus::InProgress, "in_progress"),
        (ToolStatus::Completed, "completed"),
        (ToolStatus::Failed, "failed"),
    ] {
        assert_eq!(json(&status), expected);
    }
}

#[test]
fn the_initialize_response_reports_the_version_and_names_the_agent() {
    let response = InitializeResponse {
        protocol_version: PROTOCOL_VERSION,
        agent_capabilities: AgentCapabilities::default(),
        agent_info: Implementation {
            name: "archon".into(),
            version: "1.0.0".into(),
        },
        auth_methods: Vec::new(),
    };

    let value = json(&response);
    assert_eq!(value["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(value["agentInfo"]["name"], "archon");
    assert_eq!(value["agentCapabilities"]["loadSession"], false);
    assert_eq!(
        value["authMethods"],
        serde_json::json!([]),
        "an empty list says 'nothing to log into'; omitting it says nothing"
    );
}

#[test]
fn a_new_session_request_parses_the_clients_working_directory() {
    let request: NewSessionRequest = serde_json::from_value(serde_json::json!({
        "cwd": "/home/u/project",
        "mcpServers": []
    }))
    .expect("parses");

    assert_eq!(request.cwd, "/home/u/project");
}

#[test]
fn a_prompt_request_reads_its_text_blocks() {
    let request: PromptRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "sess_1",
        "prompt": [{ "type": "text", "text": "fix the build" }]
    }))
    .expect("parses");

    assert_eq!(request.session_id, "sess_1");
    assert_eq!(request.prompt[0].as_text(), Some("fix the build"));
}

/// An image block from a client that supports more than this agent does must
/// not fail the whole prompt — the text alongside it is still answerable.
#[test]
fn an_unsupported_content_block_does_not_fail_the_prompt() {
    let request: PromptRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "sess_1",
        "prompt": [
            { "type": "image", "data": "...", "mimeType": "image/png" },
            { "type": "text", "text": "what is this" }
        ]
    }))
    .expect("an unknown block is skipped, not fatal");

    assert_eq!(request.prompt.len(), 2);
    assert_eq!(request.prompt[0].as_text(), None);
    assert_eq!(request.prompt[1].as_text(), Some("what is this"));
}

/// The doubly-tagged outcome is the protocol's shape, and the easiest thing to
/// get wrong.
#[test]
fn a_selected_permission_outcome_is_read_from_the_inner_option_id() {
    let response: RequestPermissionResponse = serde_json::from_value(serde_json::json!({
        "outcome": { "outcome": "selected", "optionId": "allow-once" }
    }))
    .expect("parses");

    assert_eq!(response.selected(), Some("allow-once"));
}

#[test]
fn a_cancelled_permission_outcome_selects_nothing() {
    let response: RequestPermissionResponse =
        serde_json::from_value(serde_json::json!({ "outcome": { "outcome": "cancelled" } }))
            .expect("parses");

    assert_eq!(response.selected(), None);
}

#[test]
fn permission_option_kinds_use_the_protocols_spelling() {
    assert_eq!(json(&PermissionOptionKind::AllowOnce), "allow_once");
    assert_eq!(json(&PermissionOptionKind::AllowAlways), "allow_always");
    assert_eq!(json(&PermissionOptionKind::RejectOnce), "reject_once");
    assert_eq!(json(&PermissionOptionKind::RejectAlways), "reject_always");
}

/// The kind picks the icon an editor shows. Getting `Bash` wrong would
/// understate what is about to happen, so anything unrecognised falls to
/// `Other` rather than to something reassuring.
#[test]
fn tool_kinds_are_classified_and_unknown_tools_are_not_flattered() {
    assert_eq!(ToolKind::for_tool("Read"), ToolKind::Read);
    assert_eq!(ToolKind::for_tool("Edit"), ToolKind::Edit);
    assert_eq!(ToolKind::for_tool("Grep"), ToolKind::Search);
    assert_eq!(ToolKind::for_tool("Bash"), ToolKind::Execute);
    assert_eq!(ToolKind::for_tool("TerminalWrite"), ToolKind::Execute);
    assert_eq!(ToolKind::for_tool("WebFetch"), ToolKind::Fetch);
    assert_eq!(ToolKind::for_tool("SomethingNew"), ToolKind::Other);
}
