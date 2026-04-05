//! Team and member configuration types for TASK-CLI-312.

use serde::{Deserialize, Serialize};

/// A single agent member configuration within a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberConfig {
    /// Role name (used as the inbox identifier, e.g. "coder", "reviewer").
    pub role: String,
    /// System prompt for this agent.
    pub system_prompt: String,
    /// Optional model override (falls back to session default if None).
    pub model: Option<String>,
    /// Tool names available to this agent.
    pub tools: Vec<String>,
}

/// A complete team definition — serialized to `team.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    /// Unique team identifier (UUID or user-supplied string).
    pub id: String,
    /// Human-readable team name.
    pub name: String,
    /// All member configurations.
    pub members: Vec<MemberConfig>,
}
