//! Router tests (#184 M1).
//!
//! These cover the thing the old router could not be tested for at all: path
//! selection. The extraction exists so both loops share one implementation, so
//! the tests exercise it directly rather than through either loop.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use archon_tools::agent_tool::SubagentRequest;
use archon_tools::send_message::SendMessageRequest;
use serde_json::json;
use tokio::sync::Mutex;

use super::*;

/// Records what the host was asked to do, so tests can assert on side effects
/// rather than only on the returned text.
#[derive(Default)]
struct RecordingHost {
    delivered: StdMutex<Vec<(String, String)>>,
    resume_reply: Option<String>,
    resumed: StdMutex<Vec<String>>,
}

impl RecordingHost {
    /// A host that cannot resume — the subagent side.
    fn without_resume() -> Self {
        Self::default()
    }

    /// A host that can resume — the main agent side.
    fn with_resume(reply: &str) -> Self {
        Self {
            resume_reply: Some(reply.to_string()),
            ..Self::default()
        }
    }

    fn delivered_to(&self) -> Vec<String> {
        self.delivered
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl RouterHost for RecordingHost {
    async fn on_delivered(&self, target_id: &str, message: &str) {
        self.delivered
            .lock()
            .unwrap()
            .push((target_id.to_string(), message.to_string()));
    }

    async fn resume_stopped_agent(&self, agent_id: &str, _message: &str) -> Option<ToolResult> {
        let reply = self.resume_reply.as_ref()?;
        self.resumed.lock().unwrap().push(agent_id.to_string());
        Some(ToolResult::success(reply.clone()))
    }
}

fn sample_request() -> SubagentRequest {
    serde_json::from_value(json!({"prompt": "p", "max_turns": 5, "timeout_secs": 60}))
        .expect("SubagentRequest fixture")
}

/// A manager with one running agent, plus its id.
fn manager_with_running_agent() -> (Arc<Mutex<SubagentManager>>, String) {
    let mut mgr = SubagentManager::new(4);
    let id = mgr.register(sample_request()).expect("register");
    (Arc::new(Mutex::new(mgr)), id)
}

fn req(to: &str, message_type: &str) -> SendMessageRequest {
    serde_json::from_value(json!({
        "to": to,
        "message": "hello",
        "summary": "s",
        "message_type": message_type,
        "request_id": "req-1",
        "approve": true,
    }))
    .expect("SendMessageRequest fixture")
}

fn lead_ctx(manager: Arc<Mutex<SubagentManager>>) -> RouterContext {
    RouterContext::new(manager, SenderIdentity::Lead)
}

fn child_ctx(manager: Arc<Mutex<SubagentManager>>, lead_id: Option<&str>) -> RouterContext {
    RouterContext::new(
        manager,
        SenderIdentity::Subagent {
            id: "subagent-child".into(),
            lead_id: lead_id.map(str::to_string),
        },
    )
}

// --- Path A: running target ------------------------------------------------

#[tokio::test]
async fn a_running_target_is_queued_not_resumed() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::with_resume("should not be used");

    let out = route(&lead_ctx(Arc::clone(&manager)), &host, &req(&id, "text")).await;

    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("queued for delivery"));
    assert!(host.resumed.lock().unwrap().is_empty(), "must not resume");
    assert_eq!(
        manager.lock().await.drain_pending_messages(&id),
        vec!["hello".to_string()]
    );
}

/// The regression M1 exists for: a subagent's message is actually routed,
/// rather than handed back to it as its own tool result.
#[tokio::test]
async fn a_subagent_can_route_a_message_to_a_peer() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), None),
        &host,
        &req(&id, "text"),
    )
    .await;

    assert!(!out.is_error, "{}", out.content);
    assert_eq!(host.delivered_to(), vec![id.clone()]);
    assert_eq!(manager.lock().await.pending_message_count(&id), 1);
}

// --- `lead` resolution -----------------------------------------------------

/// `lead` resolves from the sender's identity, never from the message.
#[tokio::test]
async fn lead_resolves_to_the_spawning_agent() {
    let (manager, parent) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), Some(&parent)),
        &host,
        &req("lead", "text"),
    )
    .await;

    assert!(!out.is_error, "{}", out.content);
    assert_eq!(host.delivered_to(), vec![parent]);
}

/// The lead is not a subagent, so it has no registry entry and `is_running`
/// reports it dead. Without the lead bypass every child-to-parent message would
/// be refused as "not running" — the delivery half of M1 would be unreachable
/// even with the address in place.
#[tokio::test]
async fn a_message_to_the_lead_is_queued_even_though_it_is_not_registered() {
    let mgr = Arc::new(Mutex::new(SubagentManager::new(4)));
    let host = RecordingHost::without_resume();

    assert!(
        !mgr.lock().await.is_running(LEAD_QUEUE_ID),
        "the lead is deliberately not a registered subagent"
    );

    let out = route(
        &child_ctx(Arc::clone(&mgr), Some(LEAD_QUEUE_ID)),
        &host,
        &req("lead", "text"),
    )
    .await;

    assert!(!out.is_error, "{}", out.content);
    assert_eq!(
        mgr.lock().await.drain_pending_messages(LEAD_QUEUE_ID),
        vec!["hello".to_string()],
        "the lead's inbox is where the main loop drains from"
    );
}

/// Backpressure applies to the lead's inbox too — a child cannot flood its
/// parent just because the parent is always reachable.
#[tokio::test]
async fn the_leads_inbox_is_bounded_like_any_other() {
    let mgr = Arc::new(Mutex::new(SubagentManager::new(4)));
    let host = RecordingHost::without_resume();
    let mut ctx = child_ctx(Arc::clone(&mgr), Some(LEAD_QUEUE_ID));
    ctx.max_pending = 1;

    assert!(!route(&ctx, &host, &req("lead", "text")).await.is_error);
    let out = route(&ctx, &host, &req("lead", "text")).await;

    assert!(out.is_error, "{}", out.content);
    assert_eq!(mgr.lock().await.pending_message_count(LEAD_QUEUE_ID), 1);
}

#[tokio::test]
async fn the_lead_itself_has_no_lead_to_address() {
    let (manager, _) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(&lead_ctx(manager), &host, &req("lead", "text")).await;

    assert!(out.is_error);
    assert!(out.content.contains("no lead"), "{}", out.content);
}

// --- Decision frames: the security invariant -------------------------------

/// A child forging approval is the attack this guards. The frame must be
/// dropped before it reaches a queue — a delivered frame is consent.
#[tokio::test]
async fn a_child_may_not_send_a_plan_approval_response() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), None),
        &host,
        &req(&id, "plan_approval_response"),
    )
    .await;

    assert!(out.is_error, "{}", out.content);
    assert!(out.content.contains("only be sent by the lead"));
    assert_eq!(
        manager.lock().await.pending_message_count(&id),
        0,
        "a refused decision frame must never be queued"
    );
    assert!(host.delivered_to().is_empty());
}

#[tokio::test]
async fn a_child_may_not_send_a_shutdown_response() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), None),
        &host,
        &req(&id, "shutdown_response"),
    )
    .await;

    assert!(out.is_error);
    assert_eq!(manager.lock().await.pending_message_count(&id), 0);
}

#[tokio::test]
async fn the_lead_may_send_a_decision_frame() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &lead_ctx(Arc::clone(&manager)),
        &host,
        &req(&id, "plan_approval_response"),
    )
    .await;

    assert!(!out.is_error, "{}", out.content);
    let queued = manager.lock().await.drain_pending_messages(&id);
    assert_eq!(queued.len(), 1);
    assert!(queued[0].contains("archon_structured_message"));
}

/// `shutdown_request` carries no `approve`, so it is a request rather than
/// consent and a child may send one.
#[tokio::test]
async fn a_child_may_send_a_shutdown_request() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), None),
        &host,
        &req(&id, "shutdown_request"),
    )
    .await;

    assert!(!out.is_error, "{}", out.content);
}

// --- Queue backpressure ----------------------------------------------------

/// An unbounded queue turns two agents talking past each other into a memory
/// leak with no symptom. Refusing at the sender makes it visible immediately.
#[tokio::test]
async fn a_full_inbox_refuses_further_messages() {
    let (manager, id) = manager_with_running_agent();
    let host = RecordingHost::without_resume();
    let mut ctx = lead_ctx(Arc::clone(&manager));
    ctx.max_pending = 2;

    for _ in 0..2 {
        assert!(!route(&ctx, &host, &req(&id, "text")).await.is_error);
    }

    let out = route(&ctx, &host, &req(&id, "text")).await;
    assert!(out.is_error, "{}", out.content);
    assert!(out.content.contains("undelivered messages"));
    assert_eq!(
        manager.lock().await.pending_message_count(&id),
        2,
        "the refused message must not be queued"
    );
}

// --- Stopped targets -------------------------------------------------------

#[tokio::test]
async fn a_stopped_target_is_resumed_where_the_host_can() {
    let (manager, id) = manager_with_running_agent();
    manager
        .lock()
        .await
        .complete(&id, "finished".into())
        .expect("complete");
    let host = RecordingHost::with_resume("resumed and answered");

    let out = route(&lead_ctx(Arc::clone(&manager)), &host, &req(&id, "text")).await;

    assert!(!out.is_error, "{}", out.content);
    assert_eq!(out.content, "resumed and answered");
    assert_eq!(host.resumed.lock().unwrap().as_slice(), &[id]);
}

/// A subagent host cannot resume, so it reports the target unreachable rather
/// than nesting a whole agent run inside its own tool round.
#[tokio::test]
async fn a_host_without_resume_reports_the_target_stopped() {
    let (manager, id) = manager_with_running_agent();
    manager
        .lock()
        .await
        .complete(&id, "finished".into())
        .expect("complete");
    let host = RecordingHost::without_resume();

    let out = route(
        &child_ctx(Arc::clone(&manager), None),
        &host,
        &req(&id, "text"),
    )
    .await;

    assert!(out.is_error, "{}", out.content);
    assert!(out.content.contains("not running"), "{}", out.content);
}

/// A decision frame is an answer to a question the target asked. Restarting a
/// stopped agent to hand it an answer it is no longer waiting for is not
/// delivery, so this path deliberately has no resume fallback.
#[tokio::test]
async fn a_decision_frame_is_never_resumed() {
    let (manager, id) = manager_with_running_agent();
    manager
        .lock()
        .await
        .complete(&id, "finished".into())
        .expect("complete");
    let host = RecordingHost::with_resume("should not be used");

    let out = route(
        &lead_ctx(Arc::clone(&manager)),
        &host,
        &req(&id, "shutdown_response"),
    )
    .await;

    assert!(out.is_error);
    assert!(host.resumed.lock().unwrap().is_empty());
}

// --- Pass-through and parsing ----------------------------------------------

#[tokio::test]
async fn non_send_message_results_pass_through_untouched() {
    let (manager, _) = manager_with_running_agent();
    let host = RecordingHost::without_resume();
    let original = ToolResult::success("some other tool's output");

    let out = maybe_route_send_message(&lead_ctx(manager), &host, "Read", original.clone()).await;

    assert_eq!(out.content, original.content);
    assert!(host.delivered_to().is_empty());
}

#[tokio::test]
async fn an_errored_send_message_result_passes_through_untouched() {
    let (manager, _) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = maybe_route_send_message(
        &lead_ctx(manager),
        &host,
        "SendMessage",
        ToolResult::error("validation failed"),
    )
    .await;

    assert!(out.is_error);
    assert!(out.content.contains("validation failed"), "{}", out.content);
}

#[tokio::test]
async fn an_unparseable_send_message_result_is_reported() {
    let (manager, _) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = maybe_route_send_message(
        &lead_ctx(manager),
        &host,
        "SendMessage",
        ToolResult::success("not json"),
    )
    .await;

    assert!(out.is_error);
    assert!(out.content.contains("Failed to parse"));
}

/// A mistyped agent name used to surface as "no transcript found", reporting a
/// name-resolution failure as a missing-transcript failure.
#[tokio::test]
async fn an_unknown_target_says_so_rather_than_blaming_the_transcript() {
    let (manager, _) = manager_with_running_agent();
    let host = RecordingHost::without_resume();

    let out = route(&lead_ctx(manager), &host, &req("reviewr", "text")).await;

    assert!(out.is_error);
    assert!(
        out.content.contains("No agent 'reviewr' is known"),
        "{}",
        out.content
    );
}
