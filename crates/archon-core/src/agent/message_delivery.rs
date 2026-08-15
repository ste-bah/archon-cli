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
    /// Only the main agent does this. The history travels to the runner through
    /// `pending_resume_messages` rather than as an argument, which is why this
    /// cannot simply be a free function.
    async fn resume_stopped_agent(&self, agent_id: &str, message: &str) -> Option<ToolsResult> {
        let ctx =
            crate::agents::transcript::AgentTranscriptStore::new(&self.agent.config.session_id)
                .and_then(|store| {
                    crate::agents::transcript::load_resume_context(&store, agent_id)
                })?;

        tracing::info!(
            agent_id = %agent_id,
            agent_type = %ctx.agent_type,
            history_len = ctx.messages.len(),
            "Resuming agent from transcript"
        );

        let resume_request = archon_tools::agent_tool::SubagentRequest {
            prompt: message.to_string(),
            model: None,
            allowed_tools: Vec::new(),
            max_turns: archon_tools::agent_tool::SubagentRequest::DEFAULT_MAX_TURNS,
            timeout_secs: archon_tools::agent_tool::SubagentRequest::DEFAULT_TIMEOUT_SECS,
            subagent_type: Some(ctx.agent_type),
            run_in_background: true,
            cwd: None,
            isolation: None,
            provider_env: None,
        };

        // Keyed by the agent being resumed, so two concurrent resumes cannot
        // hand each other's transcripts to the wrong runner (#184 M1).
        self.agent
            .pending_resume_messages
            .lock()
            .await
            .insert(agent_id.to_string(), ctx.messages);

        let tool_ctx = archon_tools::tool::ToolContext {
            working_dir: self.agent.config.working_dir.clone(),
            session_id: self.agent.config.session_id.clone(),
            // Parent-side context: `agent_id` reaches the child through the
            // executor, not through here.
            subagent_id: None,
            mode: archon_tools::tool::AgentMode::Normal,
            extra_dirs: vec![],
            in_fork: crate::agents::built_in::is_in_fork_child_by_messages(
                &self.agent.state.messages,
            ),
            nested: false,
            cancel_parent: self.agent.config.cancel_token.clone(),
            sandbox: self.agent.config.sandbox.clone(),
            activity_sink: self.agent.provider_model_activity_sink(&self.active_model),
            tool_run_parent_action_id: self.agent.guardrail_action_id.clone(),
            tool_run_tool_use_id: None,
            tool_run_attempt: 0,
            tool_run_admission: self.agent.tool_run_admission_callback.clone(),
            tool_run_outcome: self.agent.tool_run_outcome_callback.clone(),
        };

        let outcome = archon_tools::agent_tool::run_subagent(
            agent_id.to_string(),
            resume_request,
            tokio_util::sync::CancellationToken::new(),
            tool_ctx,
        )
        .await;

        Some(match outcome {
            archon_tools::subagent_executor::SubagentOutcome::Completed(text) => {
                ToolsResult::success(text)
            }
            archon_tools::subagent_executor::SubagentOutcome::Failed(err) => {
                ToolsResult::error(err)
            }
            archon_tools::subagent_executor::SubagentOutcome::AutoBackgrounded => {
                ToolsResult::success(format!(
                    "Subagent '{agent_id}' auto-backgrounded. Still running — use SendMessage to check status."
                ))
            }
            archon_tools::subagent_executor::SubagentOutcome::Cancelled => {
                ToolsResult::error("subagent cancelled")
            }
        })
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
