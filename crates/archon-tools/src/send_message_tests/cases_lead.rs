//! Addressing the lead (#184 M1).
//!
//! A subagent reporting upward is the point of the coordination layer, and it
//! was refused at this tool before the router ever saw it. `lead` is now legal
//! — but only from a subagent, and only as an alias the router resolves from
//! the sender's own identity. The model never gets to assert who its parent is.

use super::*;

/// The regression this exists for: a subagent could not address its parent at
/// all, so child -> lead delivery was unreachable no matter what the router did.
#[test]
fn a_subagent_may_address_lead() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "lead",
        "message": "found the failing test",
        "summary": "report finding upward"
    });

    let request = tool
        .validate_and_build(&input, &make_subagent_ctx())
        .expect("a subagent must be able to address its lead");
    assert_eq!(request.to, "lead");
}

/// The top-level agent has no lead. `subagent_id` is `None` there, so the alias
/// would resolve to the sender itself — an agent messaging itself in a loop.
#[test]
fn the_top_level_agent_may_not_address_lead() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "lead",
        "message": "hello",
        "summary": "attempt to reach lead"
    });

    let error = tool
        .validate_and_build(&input, &make_ctx())
        .expect_err("the top-level agent has no lead to address");
    assert!(
        error.to_string().contains("parent/main session"),
        "got: {error}"
    );
}

/// `main` names a *session*, not an agent, and a session is not a delivery
/// target. It stays rejected even for subagents — `lead` is the supported way
/// up, and keeping both would give two spellings with different resolution.
#[test]
fn main_stays_rejected_even_for_a_subagent() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "main",
        "message": "hello",
        "summary": "attempt to reach main"
    });

    let error = tool
        .validate_and_build(&input, &make_subagent_ctx())
        .expect_err("'main' is not an agent address");
    assert!(
        error.to_string().contains("parent/main session"),
        "got: {error}"
    );
}

/// The raw session id is likewise not an agent address, from either side.
#[test]
fn the_raw_session_id_stays_rejected_for_a_subagent() {
    let tool = SendMessageTool;
    let ctx = make_subagent_ctx();
    let input = json!({
        "to": ctx.session_id,
        "message": "hello",
        "summary": "attempt to reach the session"
    });

    let error = tool
        .validate_and_build(&input, &ctx)
        .expect_err("a session id is not an agent address");
    assert!(
        error.to_string().contains("parent/main session"),
        "got: {error}"
    );
}

/// Broadcast remains unsupported regardless of who asks.
#[test]
fn broadcast_stays_rejected_for_a_subagent() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "*",
        "message": "hello",
        "summary": "attempt to broadcast"
    });

    let error = tool
        .validate_and_build(&input, &make_subagent_ctx())
        .expect_err("broadcast is not supported");
    assert!(error.to_string().contains("Broadcast"), "got: {error}");
}

/// A subagent addressing a peer by name is unaffected by the lead exception.
#[test]
fn addressing_a_named_peer_is_unaffected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "reviewer",
        "message": "please look at this",
        "summary": "ask peer for review"
    });

    let request = tool
        .validate_and_build(&input, &make_subagent_ctx())
        .expect("named peers stay addressable");
    assert_eq!(request.to, "reviewer");
}

/// The decision-frame set is the two types carrying `approve`. The router
/// honours these only when the lead authored them, so the two definitions must
/// not drift apart.
#[test]
fn decision_frames_are_exactly_the_types_carrying_approve() {
    assert!(crate::send_message::is_decision_frame("shutdown_response"));
    assert!(crate::send_message::is_decision_frame(
        "plan_approval_response"
    ));

    // Carries no `approve`, so it is not consent — it is a request.
    assert!(!crate::send_message::is_decision_frame("shutdown_request"));
    assert!(!crate::send_message::is_decision_frame("text"));
    assert!(!crate::send_message::is_decision_frame("unknown"));
}
