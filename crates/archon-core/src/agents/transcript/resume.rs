//! Resume a stopped agent with the confinement it was spawned with (#241).
//!
//! A resume used to rebuild its request from the agent type alone. An agent
//! spawned with `workspace-boundary`, on a worktree rung or in a fixed
//! directory came back with none of them, and nothing said so. The spawn now
//! records its confinement beside the transcript, and a resume sends it again.
//! A transcript whose metadata does not record it is refused: whether that
//! agent was bounded cannot be told, and a guess of "no" is how a confined
//! agent comes back unconfined.

use archon_tools::isolation::Isolation;
use archon_tools::subagent_request::SubagentRequest;
use serde::{Deserialize, Serialize};

use super::{AgentMetadata, AgentTranscriptStore};

/// The fields of a spawn request that a resume must send again.
///
/// Every field is required when read back. A record that is missing one does
/// not parse, so the resume refuses the agent instead of filling a default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnConfinement {
    /// The isolation the spawn asked for: the request's value, else its agent
    /// definition's. `None` when neither named one.
    pub isolation: Option<String>,
    /// The rung of the isolation ladder the spawn ran on.
    pub tier: String,
    /// The absolute directory the agent started in, before any worktree.
    pub cwd: String,
    /// The paths named for reading under `workspace-boundary`.
    pub read_roots: Vec<String>,
    /// The write roots the caller declared. Empty means unconfined.
    pub write_roots: Vec<String>,
    /// The tool allowlist. Empty means the definition's tools.
    pub allowed_tools: Vec<String>,
    /// The model the request named, if it named one.
    pub model: Option<String>,
    /// The turn limit of the spawn.
    pub max_turns: u32,
    /// The timeout of the spawn, in seconds.
    pub timeout_secs: u64,
}

impl SpawnConfinement {
    /// The isolation value the resume sends.
    ///
    /// The boundary is sent as it was asked. A spawn that ran on a worktree
    /// rung is pinned to that rung, so the resume reuses its checkout and the
    /// automatic policy cannot put it back in the shared tree. A spawn on the
    /// shared rung sends what it asked for, so the policy decides as it did at
    /// spawn.
    fn resume_isolation(&self, agent_id: &str) -> Result<Option<String>, String> {
        let source = format!("the stored spawn record of agent '{agent_id}'");
        let asked = self
            .isolation
            .as_deref()
            .map(|raw| Isolation::parse(raw, &source))
            .transpose()
            .map_err(|error| error.to_string())?;
        if asked == Some(Isolation::WorkspaceBoundary) {
            return Ok(Some(Isolation::WorkspaceBoundary.as_str().to_string()));
        }
        let tier = Isolation::parse(&self.tier, &source)
            .map_err(|error| error.to_string())?
            .tier()
            .ok_or_else(|| {
                format!(
                    "{source} names '{}' as its rung, which is not a rung",
                    self.tier
                )
            })?;
        if tier.needs_worktree() {
            return Ok(Some(tier.as_str().to_string()));
        }
        Ok(asked.map(|isolation| isolation.as_str().to_string()))
    }
}

/// What a resume runs: the request and the history to start it from.
#[derive(Debug)]
pub struct ResumePlan {
    /// The request, with the spawn's confinement and limits restored.
    pub request: SubagentRequest,
    /// The transcript, to give the runner as its initial history.
    pub messages: Vec<serde_json::Value>,
}

/// Plan the resume of `agent_id`, with `message` as its next prompt.
///
/// `None` when the agent has no transcript, so there is nothing to resume.
/// `Err` when its metadata does not record the confinement it was spawned
/// with. The error names the agent, the missing fields and how to recover.
pub fn plan_resume(
    store: &AgentTranscriptStore,
    agent_id: &str,
    message: &str,
) -> Option<Result<ResumePlan, String>> {
    let messages = store.get_transcript(agent_id)?;
    let metadata = store.read_metadata(agent_id);
    let Some((agent_type, confinement)) = metadata
        .as_ref()
        .and_then(|meta| Some((meta.agent_type.clone(), meta.confinement.clone()?)))
    else {
        return Some(Err(unrecorded(store, agent_id, metadata.as_ref())));
    };
    Some(
        confinement
            .resume_isolation(agent_id)
            .map(|isolation| ResumePlan {
                request: SubagentRequest {
                    prompt: message.to_string(),
                    model: confinement.model,
                    allowed_tools: confinement.allowed_tools,
                    max_turns: confinement.max_turns,
                    timeout_secs: confinement.timeout_secs,
                    subagent_type: Some(agent_type),
                    run_in_background: true,
                    cwd: Some(confinement.cwd),
                    isolation,
                    read_roots: confinement.read_roots,
                    write_roots: confinement.write_roots,
                    provider_env: None,
                },
                messages,
            })
            .map_err(|error| format!("cannot resume agent '{agent_id}': {error}")),
    )
}

/// The refusal for an agent whose metadata does not record its confinement.
fn unrecorded(
    store: &AgentTranscriptStore,
    agent_id: &str,
    metadata: Option<&AgentMetadata>,
) -> String {
    let why = match metadata {
        None => "the metadata file is missing or unreadable, so whether the agent was \
                 confined cannot be told"
            .to_string(),
        Some(meta) => match meta.worktree_path.as_deref() {
            Some(worktree) => {
                format!("the agent ran in the worktree {worktree}, so it was confined")
            }
            None => "metadata written before this record existed cannot tell whether the \
                     agent was confined"
                .to_string(),
        },
    };
    format!(
        "cannot resume agent '{agent_id}': its metadata ({}) does not record the confinement \
         it was spawned with (isolation, tier, cwd, read_roots, write_roots); {why}. A resume \
         without them could run the agent unconfined, so it is refused. To continue, spawn a \
         new agent with the same isolation, cwd and roots and give it the transcript {} as \
         context.",
        store.metadata_path(agent_id).display(),
        store.transcript_path(agent_id).display(),
    )
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
