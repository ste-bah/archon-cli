use serde::{Deserialize, Serialize};

use crate::provider_env::ProviderEnvSource;

/// A validated request to spawn a subagent. The `AgentTool` validates
/// parameters and produces this struct so the outer agent loop can orchestrate
/// the real subagent lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub max_turns: u32,
    pub timeout_secs: u64,
    /// When set, loads a custom agent definition for this subagent.
    #[serde(default)]
    pub subagent_type: Option<String>,
    /// Per-call background override. When true, subagent runs as a background task.
    #[serde(default)]
    pub run_in_background: bool,
    /// Working directory override for the subagent.
    #[serde(default)]
    pub cwd: Option<String>,
    /// When set to "worktree", the subagent runs in an isolated git worktree.
    #[serde(default)]
    pub isolation: Option<String>,
    /// Directories this subagent may WRITE to. Empty means unconfined.
    ///
    /// Isolation says where an agent RUNS; this says what it may change, and
    /// the two are not the same question. An agent handed its own worktree is
    /// still given the checkout it was branched from to read, and while one
    /// list governed reads and writes alike, being able to read that tree meant
    /// being able to write it — which a worktree-isolated write agent did,
    /// editing five files in the canonical checkout unnoticed.
    ///
    /// Named by the caller rather than inferred from the working directory,
    /// because the correct set is not derivable here: an agent may legitimately
    /// have to write a declared artifact that lives outside its workspace
    /// altogether, and confining it to the workspace would refuse the one write
    /// the task exists to make.
    #[serde(default)]
    pub write_roots: Vec<String>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub provider_env: Option<ProviderEnvSource>,
}

impl SubagentRequest {
    /// Default maximum turns when the caller does not specify one.
    ///
    /// Effectively unlimited. archon trusts the configured LLM provider to
    /// return results and errors; runaway-loop protection is the USD budget
    /// cap (`--max-budget-usd`), not an arbitrary turn count.
    pub const DEFAULT_MAX_TURNS: u32 = Self::MAX_TURNS_HARD_CAP;

    /// Hard upper bound for `max_turns`. Effectively unlimited for normal runs.
    pub const MAX_TURNS_HARD_CAP: u32 = 100_000;

    /// Default timeout in seconds: 24 hours.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 86_400;
}
