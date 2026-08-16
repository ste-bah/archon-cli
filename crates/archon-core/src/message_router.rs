//! Routing a `SendMessage` result to its target agent.
//!
//! This used to live on `impl Agent`, which meant only the main loop could
//! route. A subagent calling `SendMessage` got its own serialized request back
//! as the tool result and the model believed the message had been sent —
//! "the tools exist; the coordination layer doesn't" (#184 M1).
//!
//! The routing is the same on both sides; what differs is who is calling and
//! what they are allowed to do. That difference is carried by [`SenderIdentity`]
//! and [`RouterHost`], not by two copies of the logic.
//!
//! ## Authorship is not a claim
//!
//! The two decision frames — `shutdown_response` and `plan_approval_response` —
//! carry `approve`, so delivering one is delivering consent. They are honoured
//! **only when the lead authored them**. A peer or child sending one is dropped
//! and logged.
//!
//! This works because the sender's identity comes from the call site, never
//! from the message: `SendMessageRequest` has no author field, and if it did,
//! the model would control it. The same reasoning makes `lead` an alias the
//! router resolves rather than an address the model asserts.

use std::sync::Arc;

use archon_tools::send_message::{SendMessageRequest, is_decision_frame};
use archon_tools::tool::ToolResult;
use tokio::sync::Mutex;

use crate::subagent::SubagentManager;

/// How many undelivered messages one agent may accumulate.
///
/// The queue was an unbounded `Vec::push`. With only the main loop routing that
/// was survivable; once every agent can send, a pair of agents messaging each
/// other faster than they drain is an unbounded memory leak with no symptom
/// until it is one. Refusing at the sender is the honest failure: the model
/// learns immediately, rather than the message vanishing into a queue nobody
/// reads.
pub const MAX_PENDING_MESSAGES: usize = 64;

/// Queue id the top-level agent drains.
///
/// The lead is not a subagent, so it has no entry in the registry and
/// `is_running` would report it dead — every child-to-parent message would be
/// refused. It gets a reserved id instead, matching the sentinel the board
/// leases already use for the same reason.
pub const LEAD_QUEUE_ID: &str = "top-level-agent";

/// Who is sending, established by the call site rather than by the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderIdentity {
    /// The top-level agent. The only sender whose decision frames are honoured.
    Lead,
    /// A subagent. `lead_id` is the agent that spawned it, resolved for the
    /// reserved `lead` address; `None` when the parent is the top-level agent,
    /// which has no id in the subagent registry.
    Subagent { id: String, lead_id: Option<String> },
}

impl SenderIdentity {
    fn is_lead(&self) -> bool {
        matches!(self, Self::Lead)
    }

    fn describe(&self) -> String {
        match self {
            Self::Lead => "the lead session".to_string(),
            Self::Subagent { id, .. } => format!("subagent '{id}'"),
        }
    }
}

/// The capabilities the router needs from whoever is hosting it.
///
/// Implemented by the main agent with a real resume, and by the subagent
/// runner without one — see [`RouterHost::resume_stopped_agent`].
#[async_trait::async_trait]
pub trait RouterHost: Send + Sync {
    /// Announce a delivery. Best-effort; the message is already queued.
    async fn on_delivered(&self, target_id: &str, message: &str);

    /// Restart a stopped agent from its transcript, delivering `message` as its
    /// next prompt.
    ///
    /// `None` means this host cannot resume, and the caller reports the target
    /// as unreachable rather than pretending. The subagent side returns `None`
    /// deliberately: resuming awaits a whole subagent run inline, so doing it
    /// from inside a subagent's own tool round would nest one agent's lifetime
    /// inside another's round.
    async fn resume_stopped_agent(&self, _agent_id: &str, _message: &str) -> Option<ToolResult> {
        None
    }
}

/// Everything the router needs that is not the message itself.
pub struct RouterContext {
    pub manager: Arc<Mutex<SubagentManager>>,
    pub sender: SenderIdentity,
    pub max_pending: usize,
}

impl RouterContext {
    pub fn new(manager: Arc<Mutex<SubagentManager>>, sender: SenderIdentity) -> Self {
        Self {
            manager,
            sender,
            max_pending: MAX_PENDING_MESSAGES,
        }
    }
}

/// Intercept a successful `SendMessage` tool result and route it.
///
/// Anything else passes through untouched, which is what makes this safe to
/// call on every tool result from either loop.
pub async fn maybe_route_send_message(
    ctx: &RouterContext,
    host: &dyn RouterHost,
    tool_name: &str,
    result: ToolResult,
) -> ToolResult {
    if result.is_error || tool_name != "SendMessage" {
        return result;
    }

    match serde_json::from_str::<SendMessageRequest>(&result.content) {
        Ok(req) => route(ctx, host, &req).await,
        Err(e) => ToolResult::error(format!("Failed to parse SendMessage result: {e}")),
    }
}

async fn route(ctx: &RouterContext, host: &dyn RouterHost, req: &SendMessageRequest) -> ToolResult {
    // Consent first: a rejected decision frame must not reach a queue, and the
    // check cannot depend on anything in the message.
    if is_decision_frame(&req.message_type) && !ctx.sender.is_lead() {
        tracing::warn!(
            sender = %ctx.sender.describe(),
            message_type = %req.message_type,
            target = %req.to,
            "dropping a decision frame that the lead did not author"
        );
        return ToolResult::error(format!(
            "'{}' carries an approval decision and may only be sent by the lead session. \
             {} cannot approve on the lead's behalf.",
            req.message_type,
            ctx.sender.describe(),
        ));
    }

    match req.message_type.as_str() {
        "text" => route_text(ctx, host, req).await,
        "shutdown_request" => route_shutdown_request(ctx, req).await,
        "shutdown_response" | "plan_approval_response" => {
            route_decision_frame(ctx, host, req).await
        }
        other => ToolResult::error(format!("Unknown message_type: {other}")),
    }
}

/// Resolve `to` into an agent id.
///
/// `lead` is resolved from the sender's own identity — a child cannot assert
/// who its parent is. Everything else goes through the name registry, then
/// falls back to treating the target as a raw id.
async fn resolve_target(ctx: &RouterContext, to: &str) -> Option<Target> {
    if to == archon_tools::send_message::LEAD_ADDRESS {
        return match &ctx.sender {
            SenderIdentity::Subagent { lead_id, .. } => lead_id.clone().map(Target::lead),
            SenderIdentity::Lead => None,
        };
    }

    let mgr = ctx.manager.lock().await;
    if let Some(id) = mgr.resolve_name(to) {
        return Some(Target::agent(id.to_string()));
    }
    drop(mgr);

    archon_tools::send_message::is_valid_agent_id(to).then(|| Target::agent(to.to_string()))
}

/// A resolved destination.
struct Target {
    id: String,
    /// The lead is not in the subagent registry, so `is_running` would report
    /// it dead and every child->parent message would be refused. It is running
    /// by construction: it is the agent that spawned the sender and is waiting
    /// on it.
    is_lead: bool,
}

impl Target {
    fn agent(id: String) -> Self {
        Self { id, is_lead: false }
    }

    fn lead(id: String) -> Self {
        Self { id, is_lead: true }
    }
}

async fn route_text(
    ctx: &RouterContext,
    host: &dyn RouterHost,
    req: &SendMessageRequest,
) -> ToolResult {
    let Some(target) = resolve_target(ctx, &req.to).await else {
        return unresolvable(ctx, &req.to);
    };

    let running = target.is_lead || ctx.manager.lock().await.is_running(&target.id);

    // Attribute the message when both ends are on a team. Without it a member
    // reading "please review src/foo.rs" cannot tell who asked, so it cannot
    // reply — the one thing a team needs it to be able to do (#184 M5).
    let message = team_envelope(ctx, &target, req).unwrap_or_else(|| req.message.clone());

    if running {
        if let Err(full) = enqueue(ctx, &target.id, message.clone()).await {
            return full;
        }
        host.on_delivered(&target.id, &message).await;
        return ToolResult::success(format!(
            "Message queued for delivery to {} at its next tool round.",
            req.to
        ));
    }

    // Not running: resume it from its transcript, where the host can.
    if let Some(outcome) = host.resume_stopped_agent(&target.id, &message).await {
        host.on_delivered(&target.id, &message).await;
        return outcome;
    }

    stopped_target_error(ctx, req, &target.id).await
}

/// Wrap a member-to-member message so the recipient knows who wrote it.
///
/// `None` when this is not team traffic — no team active, or the recipient is
/// not on it — which leaves ordinary lead-to-subagent messaging exactly as it
/// was. The sender's role comes from its seat on the roster, never from the
/// message: `SendMessageRequest` has no author field, and if it had one the
/// model would control it.
fn team_envelope(ctx: &RouterContext, target: &Target, req: &SendMessageRequest) -> Option<String> {
    use archon_tools::team_roster;

    let members = team_roster::members();
    if members.is_empty() {
        return None;
    }
    let role_of = |agent_id: &str| {
        members
            .iter()
            .find(|m| m.agent_id.as_deref() == Some(agent_id))
            .map(|m| m.role.clone())
    };

    let from = match &ctx.sender {
        SenderIdentity::Lead => archon_tools::send_message::LEAD_ADDRESS.to_string(),
        SenderIdentity::Subagent { id, .. } => role_of(id)?,
    };
    // A lead target is the session itself, which holds no seat.
    let to = if target.is_lead {
        archon_tools::send_message::LEAD_ADDRESS.to_string()
    } else {
        role_of(&target.id)?
    };

    Some(
        archon_tools::team_message::TeamMessage::now(
            from,
            to,
            req.message.clone(),
            archon_tools::team_message::MessageType::Chat,
        )
        .render(),
    )
}

async fn route_shutdown_request(ctx: &RouterContext, req: &SendMessageRequest) -> ToolResult {
    let Some(target) = resolve_target(ctx, &req.to).await else {
        return unresolvable(ctx, &req.to);
    };
    let target_id = target.id;

    if ctx.manager.lock().await.request_shutdown(&target_id) {
        ToolResult::success(format!("Shutdown requested for agent '{}'", req.to))
    } else {
        ToolResult::error(format!("Agent '{}' not found or not running", req.to))
    }
}

async fn route_decision_frame(
    ctx: &RouterContext,
    host: &dyn RouterHost,
    req: &SendMessageRequest,
) -> ToolResult {
    let Some(target) = resolve_target(ctx, &req.to).await else {
        return unresolvable(ctx, &req.to);
    };
    let target_id = target.id;

    // Deliberately no resume fallback, unlike text. A decision frame answers a
    // question the target asked; restarting a stopped agent to hand it an
    // answer it is no longer waiting for is not delivery.
    if !target.is_lead && !ctx.manager.lock().await.is_running(&target_id) {
        return ToolResult::error(format!(
            "Agent '{}' not running — cannot deliver structured response",
            req.to
        ));
    }

    let envelope = archon_tools::send_message::build_structured_envelope(req);
    if let Err(full) = enqueue(ctx, &target_id, envelope).await {
        return full;
    }

    let summary = format!(
        "[{}] request_id={}",
        req.message_type,
        req.request_id.as_deref().unwrap_or("")
    );
    host.on_delivered(&target_id, &summary).await;

    ToolResult::success(format!("{} delivered to {}", req.message_type, req.to))
}

/// Queue a message, refusing rather than growing without bound.
async fn enqueue(ctx: &RouterContext, agent_id: &str, message: String) -> Result<(), ToolResult> {
    let mut mgr = ctx.manager.lock().await;
    if mgr.pending_message_count(agent_id) >= ctx.max_pending {
        return Err(ToolResult::error(format!(
            "Agent '{agent_id}' already has {} undelivered messages — refusing to queue more. \
             It is not draining its inbox; wait for it to make progress.",
            ctx.max_pending
        )));
    }
    mgr.queue_pending_message(agent_id, message);
    Ok(())
}

fn unresolvable(ctx: &RouterContext, to: &str) -> ToolResult {
    if to == archon_tools::send_message::LEAD_ADDRESS {
        return ToolResult::error(
            "There is no lead to address: this is the top-level agent.".to_string(),
        );
    }
    let _ = ctx;
    ToolResult::error(format!(
        "Unknown agent '{to}' -- not in name registry and not a valid agent ID"
    ))
}

/// Why a target that resolved is nonetheless unreachable.
///
/// The old message said only "no transcript found", which was also what a
/// mistyped agent name produced — a name-resolution failure reported as a
/// missing-transcript failure. Saying which of the two happened is the
/// difference between "fix your typo" and "that agent is gone".
async fn stopped_target_error(
    ctx: &RouterContext,
    req: &SendMessageRequest,
    agent_id: &str,
) -> ToolResult {
    let state = {
        let mgr = ctx.manager.lock().await;
        mgr.get_status(agent_id)
            .map(|info| (format!("{:?}", info.status), info.result.clone()))
    };

    match state {
        Some((status, result)) => {
            let detail = result.unwrap_or_else(|| "none".into());
            ToolResult::error(format!(
                "Agent '{}' is not running (status: {status}) and could not be resumed. \
                 Last result: {detail}",
                req.to
            ))
        }
        None => ToolResult::error(format!(
            "No agent '{}' is known to this session, and no transcript was found for it. \
             If that was meant to be an agent name, check it against the running agents.",
            req.to
        )),
    }
}

#[cfg(test)]
#[path = "message_router_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "message_router_team_tests.rs"]
mod team_tests;
