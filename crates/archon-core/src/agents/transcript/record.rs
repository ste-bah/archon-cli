//! The spawn record a resume restores an agent from (#241).
//!
//! Every field is required when read back, and none has a default. A value
//! that means "nothing" is spelled out (`"unset"`, `null`, an empty list)
//! rather than left out, so a key that is missing fails the parse and the
//! resume refuses. Reading a gap as "nothing asked for" is how a bounded
//! agent comes back unbounded.

use archon_tools::isolation::{Isolation, IsolationTier};
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::ToolContext;
use serde::{Deserialize, Deserializer, Serialize};

/// The confinement and limits of one spawn, which a resume must restore
/// exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnConfinement {
    /// What the spawn's `isolation` asked for: the request's value, else its
    /// agent definition's.
    pub isolation: RecordedIsolation,
    /// The rung of the isolation ladder the spawn ran on. A resume runs on
    /// this rung or not at all.
    pub tier: IsolationTier,
    /// The absolute directory the agent started in, before any worktree.
    pub cwd: String,
    /// The paths named for reading under `workspace-boundary`.
    pub read_roots: Vec<String>,
    /// The write roots the caller declared. Empty means unconfined.
    pub write_roots: Vec<String>,
    /// The tool allowlist. Empty means the definition's tools.
    pub allowed_tools: Vec<String>,
    /// The model the request named. `null` when it named none; the key is
    /// still required.
    #[serde(deserialize_with = "required")]
    pub model: Option<String>,
    /// The turn limit of the spawn.
    pub max_turns: u32,
    /// The timeout of the spawn, in seconds.
    pub timeout_secs: u64,
    /// What the spawn's parent context confined it with.
    pub inherited: InheritedConfinement,
}

/// The `isolation` a spawn asked for, with an explicit value for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordedIsolation {
    /// Neither the request nor its definition named one.
    Unset,
    /// The shared rung, asked for by name.
    #[serde(rename = "none")]
    Shared,
    /// The worktree rung, asked for by name.
    Worktree,
    /// The worktree-with-builds rung, asked for by name.
    WorktreeWithBuilds,
    /// Confinement to the agent's own workspace.
    WorkspaceBoundary,
}

impl RecordedIsolation {
    /// The record of what a spawn asked for.
    pub fn of(asked: Option<Isolation>) -> Self {
        match asked {
            None => Self::Unset,
            Some(Isolation::Tier(IsolationTier::Shared)) => Self::Shared,
            Some(Isolation::Tier(IsolationTier::Worktree)) => Self::Worktree,
            Some(Isolation::Tier(IsolationTier::WorktreeWithBuilds)) => Self::WorktreeWithBuilds,
            Some(Isolation::WorkspaceBoundary) => Self::WorkspaceBoundary,
        }
    }

    /// What a resume asks for again.
    pub fn asked(self) -> Option<Isolation> {
        match self {
            Self::Unset => None,
            Self::Shared => Some(Isolation::Tier(IsolationTier::Shared)),
            Self::Worktree => Some(Isolation::Tier(IsolationTier::Worktree)),
            Self::WorktreeWithBuilds => Some(Isolation::Tier(IsolationTier::WorktreeWithBuilds)),
            Self::WorkspaceBoundary => Some(Isolation::WorkspaceBoundary),
        }
    }
}

/// What a spawn's parent context confined it with, beyond its request.
///
/// A resume runs from the main session's own context, which has none of
/// these. So a record that names any of them cannot be restored, and the
/// resume refuses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InheritedConfinement {
    /// A workflow run was in scope: a run store, a workflow read guard or an
    /// audit landing. Placement seals and the workflow guards follow from it.
    pub workflow: bool,
    /// The repositories whose other checkouts the agent could not write.
    pub sealed_repositories: Vec<String>,
    /// The directory names the agent could not walk into.
    pub denied_directory_names: Vec<String>,
    /// The subagent that spawned it, whose directories it inherited. `null`
    /// when the main session spawned it; the key is still required.
    #[serde(deserialize_with = "required")]
    pub parent_subagent: Option<String>,
}

impl InheritedConfinement {
    /// What `parent` hands a child it spawns.
    pub fn of(parent: &ToolContext) -> Self {
        Self {
            workflow: parent.run_store.is_some()
                || parent.workflow_read_guard.is_some()
                || parent.audit_landing.is_some(),
            sealed_repositories: parent
                .sealed_repositories
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            denied_directory_names: parent.denied_directory_names.clone(),
            parent_subagent: parent.subagent_id.clone(),
        }
    }

    /// Why a resume from the main session cannot restore this, or `None`
    /// when there is nothing to restore.
    pub fn unrestorable(&self) -> Option<String> {
        let mut why = Vec::new();
        if self.workflow {
            why.push("inside a workflow run (its run store and workflow guards)".to_string());
        }
        if !self.sealed_repositories.is_empty() {
            why.push(format!(
                "with sealed repositories ({})",
                self.sealed_repositories.join(", ")
            ));
        }
        if !self.denied_directory_names.is_empty() {
            why.push(format!(
                "with denied directory names ({})",
                self.denied_directory_names.join(", ")
            ));
        }
        if let Some(parent) = &self.parent_subagent {
            why.push(format!(
                "by subagent '{parent}', whose directories it inherited"
            ));
        }
        (!why.is_empty()).then(|| why.join("; "))
    }
}

impl SpawnConfinement {
    /// The request that runs `agent_type` again with this confinement and
    /// `prompt` as its next message.
    pub fn request(&self, agent_type: String, prompt: &str) -> SubagentRequest {
        SubagentRequest {
            prompt: prompt.to_string(),
            model: self.model.clone(),
            allowed_tools: self.allowed_tools.clone(),
            max_turns: self.max_turns,
            timeout_secs: self.timeout_secs,
            subagent_type: Some(agent_type),
            run_in_background: true,
            cwd: Some(self.cwd.clone()),
            isolation: self
                .isolation
                .asked()
                .map(|isolation| isolation.as_str().to_string()),
            read_roots: self.read_roots.clone(),
            write_roots: self.write_roots.clone(),
            provider_env: None,
        }
    }

    /// The names of the fields in which `other` differs from this record.
    pub fn differing_fields(&self, other: &Self) -> Vec<String> {
        let (Ok(serde_json::Value::Object(mine)), Ok(serde_json::Value::Object(theirs))) =
            (serde_json::to_value(self), serde_json::to_value(other))
        else {
            return vec!["the whole record".to_string()];
        };
        mine.iter()
            .filter(|(key, value)| theirs.get(*key) != Some(*value))
            .map(|(key, _)| key.clone())
            .collect()
    }
}

/// Read an `Option` whose key must be present. `serde` reads a missing
/// `Option` as `None` unless the field names its own deserializer.
fn required<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer)
}
