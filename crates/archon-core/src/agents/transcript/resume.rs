//! Disk supplies history only. Confinement is known only by the spawning process.
use super::AgentTranscriptStore;
use crate::subagent::SubagentManager;
use archon_tools::subagent_request::SubagentRequest;
use std::collections::HashMap;
use std::sync::Arc;

/// Only history and the occupancy it belongs to cross the routing seam.
/// No context clone is held here, so pending plans cannot retain collected authority.
#[derive(Debug, Clone)]
pub struct PendingResume {
    pub messages: Vec<serde_json::Value>,
    pub agent_id: String,
    pub generation: u64,
}

pub type PendingResumes = Arc<tokio::sync::Mutex<HashMap<String, PendingResume>>>;

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

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
