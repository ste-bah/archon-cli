//! Resume a stopped agent with the confinement it was spawned with (#241).
//!
//! A resume used to rebuild its request from the agent type alone. An agent
//! spawned with `workspace-boundary`, on a worktree rung or in a fixed
//! directory came back with none of them, and nothing said so. The spawn now
//! records its confinement beside the transcript ([`SpawnConfinement`]), and
//! a resume restores exactly that or refuses:
//!
//! - a record that is missing, incomplete or unreadable is refused, because
//!   whether the agent was confined cannot be told;
//! - a record whose parent context confined the agent (a workflow run, sealed
//!   repositories, denied directories, a spawning subagent) is refused,
//!   because a resume runs from the main session and cannot restore it;
//! - the record travels to the executor with the history ([`PendingResume`]),
//!   which runs the agent on its recorded rung and refuses when the rung, or
//!   any other field, would differ.

use std::collections::HashMap;
use std::sync::Arc;

use archon_tools::subagent_request::SubagentRequest;

use super::AgentTranscriptStore;
use super::record::SpawnConfinement;

/// What a resume hands the executor for one agent: the history to start
/// from and the record its run must match.
#[derive(Debug, Clone)]
pub struct PendingResume {
    /// The transcript, given to the runner as its initial history.
    pub messages: Vec<serde_json::Value>,
    /// The spawn's record. The executor pins the rung to it and refuses a run
    /// whose effective confinement differs from it.
    pub confinement: SpawnConfinement,
}

/// The executor's resume slot, keyed by the agent being resumed, so two
/// concurrent resumes cannot hand each other's history or record over.
pub type PendingResumes = Arc<tokio::sync::Mutex<HashMap<String, PendingResume>>>;

/// What a resume runs: the request, the history and the record.
#[derive(Debug)]
pub struct ResumePlan {
    /// The request, with the spawn's confinement and limits restored.
    pub request: SubagentRequest,
    /// The transcript, to give the runner as its initial history.
    pub messages: Vec<serde_json::Value>,
    /// The record the run must match.
    pub confinement: SpawnConfinement,
}

impl ResumePlan {
    /// The request to run, and what goes in the executor's resume slot.
    pub fn into_pending(self) -> (SubagentRequest, PendingResume) {
        (
            self.request,
            PendingResume {
                messages: self.messages,
                confinement: self.confinement,
            },
        )
    }
}

/// Plan the resume of `agent_id`, with `message` as its next prompt.
///
/// `None` when the agent has no transcript, so there is nothing to resume.
/// `Err` when its record is missing, incomplete or cannot be restored from
/// the main session. The error names the agent, what is missing or why, and
/// how to recover.
pub fn plan_resume(
    store: &AgentTranscriptStore,
    agent_id: &str,
    message: &str,
) -> Option<Result<ResumePlan, String>> {
    let messages = store.get_transcript(agent_id)?;
    Some(
        read_record(store, agent_id).and_then(|(agent_type, confinement)| {
            if let Some(why) = confinement.inherited.unrestorable() {
                return Err(refusal(
                    store,
                    agent_id,
                    &format!(
                        "it was spawned {why}, and a resume runs from the main session, which \
                     cannot restore that confinement"
                    ),
                ));
            }
            Ok(ResumePlan {
                request: confinement.request(agent_type, message),
                messages,
                confinement,
            })
        }),
    )
}

/// The agent type and the record from the metadata of `agent_id`.
fn read_record(
    store: &AgentTranscriptStore,
    agent_id: &str,
) -> Result<(String, SpawnConfinement), String> {
    let path = store.metadata_path(agent_id);
    let Some(meta) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    else {
        return Err(unrecorded(
            store,
            agent_id,
            "the metadata file is missing or unreadable, so whether the agent was confined \
             cannot be told",
        ));
    };
    let Some(agent_type) = meta.get("agent_type").and_then(|value| value.as_str()) else {
        return Err(incomplete(store, agent_id, "missing field `agent_type`"));
    };
    let Some(record) = meta.get("confinement").filter(|value| !value.is_null()) else {
        let why = match meta.get("worktree_path").and_then(|value| value.as_str()) {
            Some(worktree) => {
                format!("the agent ran in the worktree {worktree}, so it was confined")
            }
            None => "metadata written before this record existed cannot tell whether the \
                     agent was confined"
                .to_string(),
        };
        return Err(unrecorded(store, agent_id, &why));
    };
    let confinement =
        serde_path_to_error::deserialize::<_, SpawnConfinement>(record).map_err(|error| {
            let field = error.path().to_string();
            let detail = if field == "." {
                error.inner().to_string()
            } else {
                format!("field `{field}`: {}", error.inner())
            };
            incomplete(store, agent_id, &detail)
        })?;
    Ok((agent_type.to_string(), confinement))
}

/// The refusal for metadata that does not record the confinement at all.
fn unrecorded(store: &AgentTranscriptStore, agent_id: &str, why: &str) -> String {
    refusal(
        store,
        agent_id,
        &format!(
            "its metadata ({}) does not record the confinement it was spawned with \
             (isolation, tier, cwd, read_roots, write_roots); {why}",
            store.metadata_path(agent_id).display()
        ),
    )
}

/// The refusal for a record that does not parse in full.
fn incomplete(store: &AgentTranscriptStore, agent_id: &str, detail: &str) -> String {
    refusal(
        store,
        agent_id,
        &format!(
            "its spawn record in {} is incomplete or invalid ({detail}), and no field of it \
             is ever filled with a default",
            store.metadata_path(agent_id).display()
        ),
    )
}

fn refusal(store: &AgentTranscriptStore, agent_id: &str, why: &str) -> String {
    format!(
        "cannot resume agent '{agent_id}': {why}. A resume that does not restore the exact \
         confinement could run the agent less confined, so it is refused. To continue, spawn \
         a new agent with the same isolation, cwd and roots, from where the original was \
         spawned, and give it the transcript {} as context.",
        store.transcript_path(agent_id).display(),
    )
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
