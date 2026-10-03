//! The main agent's side of `SendMessage` routing.
//!
//! The routing itself moved to [`crate::message_router`] so subagents can use
//! it too (#184 M1). What stays here is the half only the main agent can do:
//! announcing deliveries as `AgentEvent`s. A stopped agent is never resumed
//! by message (#241).

use crate::message_router::{RouterContext, RouterHost, SenderIdentity, maybe_route_send_message};

use super::tool_types::PreflightResult;
use super::*;

/// The main agent as the router's host.
///
/// Borrows rather than clones the agent; nothing here mutates it.
struct AgentHost<'a> {
    agent: &'a Agent,
}

#[async_trait::async_trait]
impl RouterHost for AgentHost<'_> {
    async fn on_delivered(&self, target_id: &str, message: &str) {
        self.agent
            .send_event(AgentEvent::MessageSent {
                target_agent_id: target_id.to_string(),
                message: message.to_string(),
            })
            .await;
    }
}

impl Agent {
    pub(super) async fn maybe_handle_send_message_result(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
        // The post-processing chain passes the model; routing needs none now
        // that no stopped agent is resumed from here (#241).
        _active_model: &str,
    ) -> ToolResult {
        let ctx = RouterContext::new(
            std::sync::Arc::clone(&self.subagent_manager),
            // The main agent IS the lead: the only sender whose decision frames
            // are honoured.
            SenderIdentity::Lead,
        );
        let host = AgentHost { agent: self };

        maybe_route_send_message(&ctx, &host, &pre.tool_name, result).await
    }
}
