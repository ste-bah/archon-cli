//! Characterization tests for the SendMessage router (issue #184 M1).
//!
//! `Agent::maybe_handle_send_message_result` in `archon-core/src/agent/message_delivery.rs`
//! is `pub(super)` and needs a fully-built `Agent`, so it cannot be called from an
//! integration test. These tests instead pin the *observable behaviour the router depends
//! on* — the `SubagentManager` queue/name/lifecycle surface, `is_valid_agent_id`,
//! `build_structured_envelope`, and the `SendMessageTool` validation gate — so extracting
//! the router into a shared component can be proven behaviour-preserving. They describe
//! TODAY's behaviour, bugs included; nothing here is an endorsement.

use archon_core::subagent::SubagentManager;
use archon_tools::agent_tool::SubagentRequest;
use archon_tools::send_message::{SendMessageRequest, SendMessageTool, is_valid_agent_id};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use serde_json::json;

const SID: &str = "router-characterization-session";

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: SID.into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn sample_request() -> SubagentRequest {
    serde_json::from_value(json!({"prompt": "p", "max_turns": 5, "timeout_secs": 60}))
        .expect("SubagentRequest fixture")
}

/// A manager holding one freshly registered (therefore Running) agent.
fn mgr1() -> (SubagentManager, String) {
    let mut m = SubagentManager::new(4);
    let id = m.register(sample_request()).unwrap();
    (m, id)
}

/// Run the tool, assert it accepted the input, return the parsed request.
async fn ok(input: serde_json::Value) -> SendMessageRequest {
    let r = SendMessageTool.execute(input, &ctx()).await;
    assert!(!r.is_error, "expected success, got: {}", r.content);
    serde_json::from_str(&r.content).expect("valid SendMessageRequest JSON")
}

/// Run the tool, assert it rejected the input, return the error text.
async fn err(input: serde_json::Value) -> String {
    let r = SendMessageTool.execute(input, &ctx()).await;
    assert!(r.is_error, "expected error, got: {}", r.content);
    r.content
}

// --- 1. Pending-message queue semantics ------------------------------------

/// Path A queues each message; the target must observe them in send order.
#[test]
fn queue_pending_messages_preserves_fifo_order() {
    let (mut mgr, id) = mgr1();
    for m in ["first", "second", "third"] {
        mgr.queue_pending_message(&id, m.into());
    }
    let drained = mgr.drain_pending_messages(&id).join("|");
    assert_eq!(drained, "first|second|third");
}

/// Drain is take-and-clear: the router's queue must not redeliver next round.
#[test]
fn drain_pending_messages_is_take_and_clear() {
    let (mut mgr, id) = mgr1();
    mgr.queue_pending_message(&id, "only".into());
    assert_eq!(mgr.drain_pending_messages(&id).len(), 1);
    assert!(mgr.drain_pending_messages(&id).is_empty());
}

/// The manager does NO liveness check on queue, and draining an unknown id is
/// empty rather than an error — the router's `is_running` gate is the only thing
/// stopping messages from being queued into a black hole.
#[test]
fn queue_for_unregistered_id_creates_entry_and_unknown_drain_is_empty() {
    let (mut mgr, _) = mgr1();
    assert!(mgr.drain_pending_messages("no-such-agent").is_empty());
    mgr.queue_pending_message("never-registered", "orphan".into());
    assert!(!mgr.has_agent("never-registered"));
    let drained = mgr.drain_pending_messages("never-registered").join("|");
    assert_eq!(drained, "orphan");
}

/// Queues are per-agent: routing to A must never leak into B's inbox.
#[test]
fn queue_pending_messages_are_isolated_per_agent() {
    let (mut mgr, a) = mgr1();
    let b = mgr.register(sample_request()).unwrap();
    mgr.queue_pending_message(&a, "for-a".into());
    assert!(mgr.drain_pending_messages(&b).is_empty());
    assert_eq!(mgr.drain_pending_messages(&a).join("|"), "for-a");
}

// --- 2. Name registry ------------------------------------------------------

/// The registry key is the agent TYPE (run_prepare.rs registers `subagent_type`);
/// the raw id is NOT a key, so the router falls through to `is_valid_agent_id`.
#[test]
fn register_name_keys_on_agent_type_not_id() {
    let (mut mgr, id) = mgr1();
    mgr.register_name("code-reviewer".into(), id.clone());
    assert_eq!(mgr.resolve_name("code-reviewer"), Some(id.as_str()));
    assert_eq!(mgr.resolve_name(&id), None);
}

/// Two concurrent agents of the same TYPE collide: the second registration wins
/// and the first becomes permanently unaddressable by name.
#[test]
fn register_name_same_key_overwrites_previous_id() {
    let (mut mgr, first) = mgr1();
    let second = mgr.register(sample_request()).unwrap();
    mgr.register_name("explorer".into(), first.clone());
    mgr.register_name("explorer".into(), second.clone());
    assert_eq!(mgr.resolve_name("explorer"), Some(second.as_str()));
    assert_ne!(mgr.resolve_name("explorer"), Some(first.as_str()));
}

/// An unknown name resolves to None, sending the router to its raw-id fallback.
#[test]
fn resolve_name_unknown_is_none() {
    let (mgr, _) = mgr1();
    assert_eq!(mgr.resolve_name("nobody"), None);
}

// --- 3. Silent-loss points (load-bearing for the resume path) --------------

/// SILENT LOSS: cleanup discards anything queued but not yet drained.
#[test]
fn cleanup_agent_discards_queued_messages() {
    let (mut mgr, id) = mgr1();
    mgr.queue_pending_message(&id, "never-delivered".into());
    mgr.cleanup_agent(&id);
    assert!(mgr.drain_pending_messages(&id).is_empty());
}

/// SILENT LOSS: one id may carry several names, and cleanup purges EVERY one of
/// them, so a later SendMessage by name misses the registry and takes the
/// raw-id fallback instead of resolving.
#[test]
fn cleanup_agent_purges_all_names_for_that_id() {
    let (mut mgr, id) = mgr1();
    mgr.register_name("alpha".into(), id.clone());
    mgr.register_name("beta".into(), id.clone());
    assert_eq!(mgr.resolve_name("alpha"), Some(id.as_str()));
    assert_eq!(mgr.resolve_name("beta"), Some(id.as_str()));
    mgr.cleanup_agent(&id);
    assert_eq!(mgr.resolve_name("alpha"), None);
    assert_eq!(mgr.resolve_name("beta"), None);
}

/// Cleanup is scoped to one id: a sibling agent's name and queue survive.
#[test]
fn cleanup_agent_leaves_other_agents_untouched() {
    let (mut mgr, a) = mgr1();
    let b = mgr.register(sample_request()).unwrap();
    mgr.register_name("keeper".into(), b.clone());
    mgr.queue_pending_message(&b, "kept".into());
    mgr.cleanup_agent(&a);
    assert_eq!(mgr.resolve_name("keeper"), Some(b.as_str()));
    assert_eq!(mgr.drain_pending_messages(&b).join("|"), "kept");
}

/// Cleanup does NOT drop the agent entry — the router's "not running (status: ..)"
/// branch depends on `get_status` still returning Some after cleanup.
#[test]
fn cleanup_agent_keeps_the_agent_entry_and_status() {
    let (mut mgr, id) = mgr1();
    mgr.complete(&id, "done".into()).unwrap();
    mgr.cleanup_agent(&id);
    assert!(mgr.has_agent(&id));
    assert_eq!(mgr.get_status(&id).unwrap().result.as_deref(), Some("done"));
}

/// SILENT LOSS: resuming a stopped agent via `register_with_id` drops whatever was
/// queued while it was stopped (reachable only via a racing concurrent sender).
#[test]
fn register_with_id_reuse_discards_queued_messages() {
    let (mut mgr, id) = mgr1();
    mgr.complete(&id, "done".into()).unwrap();
    mgr.queue_pending_message(&id, "queued-while-stopped".into());
    mgr.register_with_id(id.clone(), sample_request()).unwrap();
    assert!(mgr.drain_pending_messages(&id).is_empty());
}

/// SILENT LOSS: resume also purges the name registry for that id, so the resumed
/// agent is nameless until run_prepare re-registers its type.
#[test]
fn register_with_id_reuse_purges_name_registry_for_that_id() {
    let (mut mgr, id) = mgr1();
    mgr.register_name("explorer".into(), id.clone());
    mgr.complete(&id, "done".into()).unwrap();
    mgr.register_with_id(id.clone(), sample_request()).unwrap();
    assert_eq!(mgr.resolve_name("explorer"), None);
}

/// A running duplicate is rejected, and the rejection preserves the queue — so an
/// AGT-025 double-resume race cannot silently eat pending messages.
#[test]
fn register_with_id_on_running_agent_errors_and_preserves_queue() {
    let (mut mgr, id) = mgr1();
    mgr.queue_pending_message(&id, "kept".into());
    assert!(mgr.register_with_id(id.clone(), sample_request()).is_err());
    assert_eq!(mgr.drain_pending_messages(&id).join("|"), "kept");
}

/// Reuse resets status to Running, flipping the router from resume back to path A.
#[test]
fn register_with_id_reuse_resets_status_to_running() {
    let (mut mgr, id) = mgr1();
    mgr.complete(&id, "done".into()).unwrap();
    assert!(!mgr.is_running(&id));
    mgr.register_with_id(id.clone(), sample_request()).unwrap();
    assert!(mgr.is_running(&id));
    assert!(mgr.get_status(&id).unwrap().result.is_none());
}

// --- 4. is_running transitions (path A vs path B/C selector) ---------------

/// Freshly registered agents are running -> path A; after `complete` they are not
/// -> the router leaves path A for the resume path.
#[test]
fn is_running_true_after_register_and_false_after_complete() {
    let (mut mgr, id) = mgr1();
    assert!(mgr.is_running(&id));
    mgr.complete(&id, "r".into()).unwrap();
    assert!(!mgr.is_running(&id));
}

/// After `mark_failed` or `mark_timed_out`, is_running is false -> resume path.
#[test]
fn is_running_false_after_mark_failed_and_mark_timed_out() {
    let (mut mgr, a) = mgr1();
    let b = mgr.register(sample_request()).unwrap();
    mgr.mark_failed(&a, "boom".into()).unwrap();
    mgr.mark_timed_out(&b).unwrap();
    assert!(!mgr.is_running(&a));
    assert!(!mgr.is_running(&b));
}

/// An id the manager never saw is not running -> router tries resume-from-disk.
#[test]
fn is_running_unknown_id_is_false() {
    let (mgr, _) = mgr1();
    assert!(!mgr.is_running("ghost"));
}

// --- 5. is_valid_agent_id — the router's raw-id fallback gate --------------

/// Empty target is rejected -> router returns "Unknown agent".
#[test]
fn is_valid_agent_id_rejects_empty() {
    assert!(!is_valid_agent_id(""));
}

/// Any ASCII space rejects -> prose targets error out instead of resuming.
#[test]
fn is_valid_agent_id_rejects_strings_containing_a_space() {
    assert!(!is_valid_agent_id("the code reviewer"));
    assert!(!is_valid_agent_id(" leading"));
    assert!(!is_valid_agent_id("trailing "));
}

/// The length gate is inclusive at 128 and counts BYTES, not chars.
#[test]
fn is_valid_agent_id_length_boundary_is_128_bytes_inclusive() {
    assert!(is_valid_agent_id(&"a".repeat(128)));
    assert!(!is_valid_agent_id(&"a".repeat(129)));
    assert!(is_valid_agent_id(&"é".repeat(64)));
    assert!(!is_valid_agent_id(&"é".repeat(65)));
}

/// SURPRISE: any ordinary word passes, and only U+0020 is checked (tabs/newlines
/// pass), so an unregistered name does NOT error — it falls through to the resume
/// path and reports "No transcript found" instead of "Unknown agent".
#[test]
fn is_valid_agent_id_accepts_ordinary_words_and_non_space_whitespace() {
    assert!(is_valid_agent_id("reviewer"));
    assert!(is_valid_agent_id("../../etc/passwd"));
    assert!(is_valid_agent_id("a\tb"));
    assert!(is_valid_agent_id("a\nb"));
}

/// UUID and prefixed-uuid shapes pass, which is the intended use.
#[test]
fn is_valid_agent_id_accepts_uuid_shapes() {
    assert!(is_valid_agent_id("550e8400-e29b-41d4-a716-446655440000"));
    assert!(is_valid_agent_id(
        "agent-550e8400-e29b-41d4-a716-446655440000"
    ));
}

// --- 7. SendMessageTool validation surface (what ever reaches the router) ---

/// Omitted message_type defaults to "text" -> router takes the text branch.
#[tokio::test]
async fn tool_defaults_message_type_to_text() {
    let req = ok(json!({"to": "r", "message": "hi", "summary": "greeting"})).await;
    assert_eq!(req.message_type, "text");
}

/// The three structured message_type values are accepted alongside "text".
#[tokio::test]
async fn tool_accepts_the_three_structured_message_types() {
    for mt in [
        "shutdown_request",
        "shutdown_response",
        "plan_approval_response",
    ] {
        let v = json!({"to": "r", "message_type": mt, "request_id": "r1", "approve": true});
        assert_eq!(ok(v).await.message_type, mt);
    }
}

/// Unknown message_type is rejected at the tool, so the router's `other =>` arm is
/// unreachable from the tool (only from hand-built JSON).
#[tokio::test]
async fn tool_rejects_unknown_message_type() {
    let e = err(json!({"to": "r", "message_type": "broadcast", "message": "x"})).await;
    assert!(e.contains("Unknown message_type"), "{e}");
}

/// request_id is required for both response types.
#[tokio::test]
async fn tool_requires_request_id_for_response_types() {
    for mt in ["shutdown_response", "plan_approval_response"] {
        let e = err(json!({"to": "r", "message_type": mt, "approve": true})).await;
        assert!(e.contains("request_id is required"), "{mt}: {e}");
    }
}

/// approve is required for both response types.
#[tokio::test]
async fn tool_requires_approve_for_response_types() {
    for mt in ["shutdown_response", "plan_approval_response"] {
        let e = err(json!({"to": "r", "message_type": mt, "request_id": "r1"})).await;
        assert!(e.contains("approve is required"), "{mt}: {e}");
    }
}

/// shutdown_request needs neither request_id nor approve.
#[tokio::test]
async fn tool_does_not_require_request_id_or_approve_for_shutdown_request() {
    ok(json!({"to": "r", "message_type": "shutdown_request"})).await;
}

/// Neither `to == "main"` nor `to == ctx.session_id` ever reaches the router.
#[tokio::test]
async fn tool_rejects_to_main_and_to_own_session_id() {
    let e = err(json!({"to": "main", "message": "x", "summary": "s"})).await;
    assert!(e.contains("parent/main session"), "{e}");
    let e = err(json!({"to": SID, "message": "x", "summary": "s"})).await;
    assert!(e.contains("parent/main session"), "{e}");
}

/// Broadcast `*` never reaches the router.
#[tokio::test]
async fn tool_rejects_broadcast_star() {
    let e = err(json!({"to": "*", "message": "x", "summary": "s"})).await;
    assert!(e.contains("Broadcast"), "{e}");
}

/// summary is validation-required for text messages, and not for structured ones.
#[tokio::test]
async fn tool_requires_summary_only_for_text_messages() {
    let e = err(json!({"to": "r", "message": "x"})).await;
    assert!(e.contains("summary is required"), "{e}");
    ok(json!({"to": "r", "message_type": "shutdown_response",
        "request_id": "r1", "approve": true}))
    .await;
}

/// message must be non-blank for text, and is optional for structured types.
#[tokio::test]
async fn tool_requires_nonblank_message_only_for_text() {
    err(json!({"to": "r", "message": "   ", "summary": "s"})).await;
    ok(json!({"to": "r", "message_type": "shutdown_request"})).await;
}

/// `to` is trimmed before it reaches the router, so " reviewer " resolves as "reviewer".
#[tokio::test]
async fn tool_trims_the_to_field() {
    let req = ok(json!({"to": "  reviewer  ", "message": "x", "summary": "s"})).await;
    assert_eq!(req.to, "reviewer");
}

/// A `to` with an interior space survives the tool but fails `is_valid_agent_id`,
/// so the router answers "Unknown agent" rather than attempting a resume.
#[tokio::test]
async fn tool_allows_interior_spaces_in_to_which_the_router_then_rejects() {
    let req = ok(json!({"to": "code reviewer", "message": "x", "summary": "s"})).await;
    assert!(!is_valid_agent_id(&req.to));
}
