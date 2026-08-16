//! Team and member configuration types for TASK-CLI-312.

use serde::{Deserialize, Serialize};

/// A single agent member configuration within a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberConfig {
    /// Role name (e.g. "coder", "reviewer"). Also the address other members
    /// send to, because a spawn's `subagent_type` is what the router's name
    /// registry resolves.
    pub role: String,
    /// System prompt for this agent.
    pub system_prompt: String,
    /// Optional model override (falls back to session default if None).
    pub model: Option<String>,
    /// Tool names available to this agent.
    pub tools: Vec<String>,
    /// The running agent filling this seat, if any (#184 M5).
    ///
    /// A declared seat with no `agent_id` is vacant. Serde-defaulted, so a
    /// `team.json` written before this field existed still loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Whether `TeamCreate` asked for this seat, as opposed to an agent
    /// arriving in a role the team never named (#184 M5).
    ///
    /// It decides what survives a departure: a declared seat outlives its
    /// occupant, an undeclared one does not. Recorded rather than inferred —
    /// guessing from an empty prompt would delete a declared seat that simply
    /// had nothing to say. Defaults to `true`, because a `team.json` written
    /// before this field existed contains only declared members.
    #[serde(default = "declared_by_default")]
    pub declared: bool,
}

fn declared_by_default() -> bool {
    true
}

impl MemberConfig {
    /// A seat the team asked for, nobody in it yet.
    pub fn declared(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            system_prompt: String::new(),
            model: None,
            tools: Vec::new(),
            agent_id: None,
            declared: true,
        }
    }

    /// A seat for an agent that arrived in a role the team never named.
    pub fn undeclared(role: impl Into<String>) -> Self {
        Self {
            declared: false,
            ..Self::declared(role)
        }
    }

    /// Whether a running agent currently fills this seat.
    pub fn is_filled(&self) -> bool {
        self.agent_id.is_some()
    }
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
