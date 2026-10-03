//! The main agent's side of `SendMessage` routing.
//!
//! The routing itself moved to [`crate::message_router`] so subagents can use
//! it too (#184 M1). What stays here is the half only the main agent can do:
//! announcing deliveries as `AgentEvent`s, and resuming a stopped agent from
//! its transcript.

use archon_tools::tool::ToolResult as ToolsResult;

use crate::message_router::{RouterContext, RouterHost, SenderIdentity, maybe_route_send_message};

use super::tool_types::PreflightResult;
use super::*;

/// The main agent as the router's host.
///
/// Borrows rather than clones the agent: every field the resume path needs is
/// either behind an `Arc` or cheap to clone at the point of use, and nothing
/// here mutates the agent.
struct AgentHost<'a> {
    agent: &'a Agent,
    active_model: String,
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

    /// Restart a stopped agent from its transcript.
    ///
    /// Only the main agent does this. The manager's process-local context is
    /// the only resume authority; the history travels with the run itself.
    async fn resume_stopped_agent(&self, agent_id: &str, message: &str) -> Option<ToolsResult> {
        let store =
            crate::agents::transcript::AgentTranscriptStore::new(&self.agent.config.session_id)?;
        // Scheduling context only. The runner uses its stored effective context.
        let tool_ctx = archon_tools::tool::ToolContext {
            working_dir: self.agent.config.working_dir.clone(),
            session_id: self.agent.config.session_id.clone(),
            // Parent-side context: `agent_id` reaches the child through the
            // executor, not through here.
            subagent_id: None,
            turn_id: Some(format!(
                "{}#{}",
                self.agent.config.session_id,
                self.agent.turn_number()
            )),
            mode: archon_tools::tool::AgentMode::Normal,
            extra_dirs: vec![],
            denied_directory_names: Vec::new(),
            write_roots: Vec::new(),
            sealed_repositories: Vec::new(),
            run_store: None,
            in_fork: crate::agents::built_in::is_in_fork_child_by_messages(
                &self.agent.state.messages,
            ),
            nested: false,
            cancel_parent: self.agent.config.cancel_token.clone(),
            sandbox: self.agent.config.sandbox.clone(),
            fs: self.agent.config.fs.clone(),
            activity_sink: self.agent.provider_model_activity_sink(&self.active_model),
            tool_run_parent_action_id: self.agent.guardrail_action_id.clone(),
            tool_run_tool_use_id: None,
            tool_run_attempt: 0,
            repeat_tool: self.agent.config.repeat_tool.clone(),
            workflow_read_guard: None,
            audit_landing: None,
            tool_run_admission: self.agent.tool_run_admission_callback.clone(),
            tool_run_outcome: self.agent.tool_run_outcome_callback.clone(),
        };
        // The turn's interrupt stops the resume while it queues and while it runs.
        let cancel = self
            .agent
            .config
            .cancel_token
            .as_ref()
            .map(tokio_util::sync::CancellationToken::child_token)
            .unwrap_or_default();
        Some(
            crate::agents::transcript::resume_agent(
                &store,
                &self.agent.subagent_manager,
                agent_id,
                message,
                tool_ctx,
                cancel,
            )
            .await,
        )
    }
}

impl Agent {
    pub(super) async fn maybe_handle_send_message_result(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
        active_model: &str,
    ) -> ToolResult {
        let ctx = RouterContext::new(
            std::sync::Arc::clone(&self.subagent_manager),
            // The main agent IS the lead: the only sender whose decision frames
            // are honoured.
            SenderIdentity::Lead,
        );
        let host = AgentHost {
            agent: self,
            active_model: active_model.to_string(),
        };

        maybe_route_send_message(&ctx, &host, &pre.tool_name, result).await
    }
}
