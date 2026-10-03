//! Disk supplies history only. Confinement is known only by the spawning process.
use super::AgentTranscriptStore;
use crate::subagent::SubagentManager;
use archon_tools::subagent_request::SubagentRequest;
use std::sync::Arc;

/// Only history and the occupancy it belongs to cross the routing seam.
/// No context clone is held here, so pending plans cannot retain collected authority.
#[derive(Debug, Clone)]
pub struct PendingResume {
    pub messages: Vec<serde_json::Value>,
    pub agent_id: String,
    pub generation: u64,
}

#[derive(Debug)]
pub struct ResumePlan {
    /// Used for scheduling and the new prompt; confinement is never rebuilt from it.
    pub request: SubagentRequest,
    pub messages: Vec<serde_json::Value>,
    agent_id: String,
    generation: u64,
}

impl ResumePlan {
    pub fn into_pending(self) -> (SubagentRequest, PendingResume) {
        (
            self.request,
            PendingResume {
                messages: self.messages,
                agent_id: self.agent_id,
                generation: self.generation,
            },
        )
    }
}

pub(crate) fn unknown_context(agent_id: &str) -> String {
    format!(
        "cannot resume agent '{agent_id}': its confinement is only known to the process that started it; start a new agent"
    )
}

/// The manager lookup is the sole authority, for confined and unconfined agents.
/// No sidecar is read, even to choose the agent type or claim it was unconfined.
pub fn plan_resume(
    store: &AgentTranscriptStore,
    manager: &SubagentManager,
    agent_id: &str,
    message: &str,
) -> Result<ResumePlan, String> {
    let info = manager
        .get_status(agent_id)
        .filter(|info| info.effective_context.is_some())
        .ok_or_else(|| unknown_context(agent_id))?;
    if info.status == crate::subagent::SubagentStatus::Running {
        return Err(format!(
            "cannot resume agent '{agent_id}': it is already running"
        ));
    }
    let context = info.effective_context.as_ref().expect("checked above");
    let messages = store.get_transcript(agent_id).ok_or_else(|| {
            format!("cannot resume agent '{agent_id}': its conversation history is unavailable; start a new agent")
        })?;
    let mut request = context.request.clone();
    request.prompt = message.into();
    request.run_in_background = false;
    Ok(ResumePlan {
        request,
        messages,
        agent_id: agent_id.into(),
        generation: info.generation,
    })
}

impl PendingResume {
    /// Run `work` (which starts this agent's run) carrying this resume. The
    /// run takes it at registration; no other run can see it, and it lives
    /// as long as the run does, however soon the caller stops waiting.
    pub async fn carry<T>(self, work: impl std::future::Future<Output = T>) -> T {
        archon_tools::subagent_resume::scope(
            archon_tools::subagent_resume::ResumeScope {
                agent_id: self.agent_id.clone(),
                payload: Arc::new(self),
            },
            work,
        )
        .await
    }

    /// The resume of `agent_id` the current run carries.
    pub(crate) fn carried(agent_id: &str) -> Option<Arc<Self>> {
        archon_tools::subagent_resume::current_for(agent_id)?
            .payload
            .downcast::<Self>()
            .ok()
    }
}

/// Resume the stopped agent `agent_id` with `message` as its next prompt, as
/// the main agent's message router does.
///
/// `cancel` is the caller's interrupt: it stops the run while it waits for
/// capacity, and the run itself, and the outcome then reads as cancelled.
pub async fn resume_agent(
    store: &AgentTranscriptStore,
    manager: &tokio::sync::Mutex<SubagentManager>,
    agent_id: &str,
    message: &str,
    tool_ctx: archon_tools::tool::ToolContext,
    cancel: tokio_util::sync::CancellationToken,
) -> archon_tools::tool::ToolResult {
    use archon_tools::subagent_executor::SubagentOutcome;
    use archon_tools::tool::ToolResult;
    let plan = match plan_resume(store, &*manager.lock().await, agent_id, message) {
        Ok(plan) => plan,
        Err(refusal) => {
            tracing::warn!(agent_id = %agent_id, %refusal, "agent resume refused");
            return ToolResult::error(refusal);
        }
    };
    tracing::info!(
        agent_id = %agent_id,
        agent_type = ?plan.request.subagent_type,
        isolation = ?plan.request.isolation,
        history_len = plan.messages.len(),
        "Resuming agent from transcript"
    );
    let (request, pending) = plan.into_pending();
    let outcome = pending
        .carry(archon_tools::agent_tool::run_subagent(
            agent_id.to_string(),
            request,
            cancel,
            tool_ctx,
        ))
        .await;
    match outcome {
        SubagentOutcome::Completed(text) => ToolResult::success(text),
        SubagentOutcome::Failed(err) => ToolResult::error(err),
        SubagentOutcome::AutoBackgrounded => ToolResult::success(format!(
            "Subagent '{agent_id}' auto-backgrounded. Still running — use SendMessage to check status."
        )),
        SubagentOutcome::Cancelled => ToolResult::error("subagent cancelled"),
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
