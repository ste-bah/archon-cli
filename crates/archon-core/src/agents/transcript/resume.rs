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
    /// Which resume put this entry in the slot, so its reservation removes
    /// only its own entry and never a later resume's.
    ticket: u64,
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
                ticket: NEXT_TICKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            },
        )
    }
}

static NEXT_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A resume's entry in the executor's slot, held while its run starts.
///
/// The executor takes the entry when it registers the run. Until then a
/// second resume of the same agent is refused rather than allowed to replace
/// the entry: the run that then found no entry would start as a new spawn
/// from the stored request, without the stored context. Dropping the
/// reservation removes an entry no run took, so it cannot reach a later run.
pub struct ResumeReservation {
    slot: PendingResumes,
    agent_id: String,
    ticket: u64,
}

/// Put `pending` in `slot`, or refuse while another resume of the same agent
/// waits there.
pub async fn reserve_resume(
    slot: &PendingResumes,
    pending: PendingResume,
) -> Result<ResumeReservation, String> {
    let mut entries = slot.lock().await;
    if entries.contains_key(&pending.agent_id) {
        return Err(format!(
            "cannot resume agent '{}': another resume of it is already starting; wait for it to finish",
            pending.agent_id
        ));
    }
    let reservation = ResumeReservation {
        slot: Arc::clone(slot),
        agent_id: pending.agent_id.clone(),
        ticket: pending.ticket,
    };
    entries.insert(pending.agent_id.clone(), pending);
    Ok(reservation)
}

impl ResumeReservation {
    fn release(entries: &mut HashMap<String, PendingResume>, agent_id: &str, ticket: u64) {
        if entries.get(agent_id).is_some_and(|entry| entry.ticket == ticket) {
            entries.remove(agent_id);
        }
    }
}

impl Drop for ResumeReservation {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.slot.try_lock() {
            Self::release(&mut entries, &self.agent_id, self.ticket);
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (slot, agent_id, ticket) = (Arc::clone(&self.slot), self.agent_id.clone(), self.ticket);
        runtime.spawn(async move {
            Self::release(&mut *slot.lock().await, &agent_id, ticket);
        });
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
