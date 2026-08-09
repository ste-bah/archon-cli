//! The IDE permission round-trip, end to end (issue #26, item 5).
//!
//! These are the tests that decide whether tools may be enabled at all.
//! Before this landed, `Agent::request_tool_permission` auto-approved
//! everything when `permission_response_rx` was `None`, so an IDE session with
//! tools would have run Bash and Write with nobody asked. The assertions below
//! are therefore about *execution*, not about notifications: a probe tool
//! records whether it actually ran, and "denied" means that flag stayed false.

mod ide_support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use archon_core::dispatch::ToolRegistry;

use ide_support::{Harness, PROBE_TOOL, ProbeTool, ScriptedProvider, text_round, tool_use_round};

/// Build a session whose model immediately asks to run the probe tool, then
/// says "done" once it has an answer.
fn probe_session(client_can_approve: bool) -> (Harness, Arc<std::sync::atomic::AtomicBool>) {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_use_round(PROBE_TOOL, "tool-1"),
        text_round("done"),
    ]));
    let (probe, executed) = ProbeTool::new();
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(probe));
    // `default` mode, not `auto`: in `auto` the agent decides for itself and
    // never asks, which is exactly the mode `ide-stdio` upgrades away from.
    let harness = Harness::with_tools(provider, tools, "default", client_can_approve);
    (harness, executed)
}

/// Run until the permission prompt arrives, and return its correlation id.
async fn prompt_until_permission(harness: &mut Harness) -> String {
    harness.prompt("do the risky thing");
    let request = harness.drain_until("archon/permissionRequest").await;
    assert_eq!(request.params["action"], PROBE_TOOL, "{request:?}");
    assert_eq!(
        request.params["sessionId"], harness.session_id,
        "the prompt must be addressed to the negotiated session"
    );
    request.params["requestId"]
        .as_str()
        .expect("permissionRequest carries a requestId")
        .to_string()
}

// ── The two assertions the whole feature exists for ──────────────────────────

/// Deny must mean the tool never ran. Not "was logged", not "was reported to
/// the IDE" — never ran.
#[tokio::test]
async fn a_denied_permission_request_does_not_execute_the_tool() {
    let (mut harness, executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    let response = harness.answer_permission(&request_id, false);
    assert_eq!(response["result"]["delivered"], true, "{response}");

    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;

    assert!(
        !executed.load(Ordering::SeqCst),
        "DENIED TOOL EXECUTED — the permission gate is fail-open"
    );
}

/// The mirror image: the same wiring, answered `true`, must actually run the
/// tool. Without this, a gate that denied everything would pass the test above.
#[tokio::test]
async fn an_approved_permission_request_executes_the_tool() {
    let (mut harness, executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    harness.answer_permission(&request_id, true);

    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;

    assert!(
        executed.load(Ordering::SeqCst),
        "an approved tool never ran; the answer did not reach the agent"
    );
}

/// The agent must *wait*, not proceed optimistically and ask afterwards.
#[tokio::test]
async fn the_agent_waits_for_a_decision_instead_of_proceeding() {
    let (mut harness, executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    // Nothing has been answered. The turn must be parked: no tool execution,
    // no further notifications, and the agent lock still held.
    harness.expect_silence(Duration::from_millis(400)).await;
    assert!(
        !executed.load(Ordering::SeqCst),
        "the tool ran before anyone approved it"
    );
    assert!(
        harness.agent.try_lock().is_err(),
        "the turn released the agent instead of waiting for a decision"
    );

    // And it is genuinely parked rather than dead: answering releases it.
    harness.answer_permission(&request_id, true);
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;
    assert!(executed.load(Ordering::SeqCst));
}

// ── Correlation and misuse ───────────────────────────────────────────────────

#[tokio::test]
async fn a_decision_for_the_wrong_request_is_refused_and_the_agent_keeps_waiting() {
    let (mut harness, executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    let response = harness.answer_permission("perm-999", true);

    assert!(response.get("result").is_none(), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("not the one awaiting a decision")),
        "{response}"
    );
    assert!(!executed.load(Ordering::SeqCst), "a stale id ran the tool");

    harness.answer_permission(&request_id, false);
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;
}

#[tokio::test]
async fn an_answer_with_nothing_pending_is_an_error_rather_than_a_quiet_success() {
    let (mut harness, _executed) = probe_session(true);

    let response = harness.answer_permission("perm-1", true);

    assert!(response.get("result").is_none(), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("no permission request")),
        "{response}"
    );
}

#[tokio::test]
async fn the_same_decision_cannot_be_delivered_twice() {
    let (mut harness, _executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;
    harness.answer_permission(&request_id, true);

    let second = harness.answer_permission(&request_id, true);

    assert!(second.get("result").is_none(), "{second}");
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;
}

// ── The decision is announced back to the IDE ────────────────────────────────

#[tokio::test]
async fn the_ide_is_told_how_the_request_resolved() {
    let (mut harness, _executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    harness.answer_permission(&request_id, false);

    let resolved = harness.drain_until("archon/permissionResolved").await;
    assert_eq!(resolved.params["granted"], false, "{resolved:?}");
    assert_eq!(resolved.params["action"], PROBE_TOOL, "{resolved:?}");
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;
}

/// The tool's denial has to reach the model as a tool result too, otherwise
/// the turn would hang waiting for output that never comes.
#[tokio::test]
async fn a_denial_is_reported_to_the_model_as_a_failed_tool_call() {
    let (mut harness, _executed) = probe_session(true);
    let request_id = prompt_until_permission(&mut harness).await;

    harness.answer_permission(&request_id, false);

    let complete = harness.drain_until("archon/toolCallComplete").await;
    assert_eq!(complete.params["isError"], true, "{complete:?}");
    assert!(
        complete.params["content"]
            .as_str()
            .is_some_and(|c| c.contains("Permission denied")),
        "{complete:?}"
    );
    harness.wait_for_idle_agent().await;
}

// ── Clients with no approval UI ──────────────────────────────────────────────

/// A client that never advertised `toolExecution` has no way to answer, so it
/// must be refused immediately rather than treated as consent — and rather
/// than hanging the session until the agent's own two-minute timeout.
#[tokio::test]
async fn a_client_without_an_approval_ui_never_gets_the_tool_run() {
    let (mut harness, executed) = probe_session(false);

    harness.prompt("do the risky thing");
    harness.drain_until("archon/permissionRequest").await;
    harness.drain_until("archon/turnComplete").await;
    harness.wait_for_idle_agent().await;

    assert!(
        !executed.load(Ordering::SeqCst),
        "a client with no approval UI got a tool run anyway"
    );
}
